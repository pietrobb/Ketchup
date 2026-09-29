//! The one source of geometric tolerances and model limits.
//!
//! A model tolerance decides whether two things are the same (coincident points, a
//! zero-length edge, parallel directions). It belongs to the document as a
//! [`TolerancePolicy`] and is passed to every check that needs it. Modules do not
//! define their own epsilons.
//!
//! The limits below bound the model space. They are not tolerances; they keep every
//! coordinate where `f64` still resolves the linear tolerance with margin.
use std::fmt;

/// Evidence identity of the policy semantics; part of validation digests.
pub const TOLERANCE_POLICY_ID: &str = "ketchup.prismatic-tolerance.v1";

/// Largest absolute model coordinate or distance, in mm (1 km). The `f64` spacing at
/// this magnitude is about 1.2e-10 mm, so the default linear tolerance stays roughly
/// a thousand representable steps wide everywhere in the model.
pub const MAX_COORDINATE_MM: f64 = 1_000_000.0;

/// Default linear tolerance in mm; the same value as OCCT `Precision::Confusion()`, so
/// the exact kernel and the Rust checks agree on coincidence.
pub const DEFAULT_LINEAR_TOLERANCE_MM: f64 = 1.0e-7;

/// Default angular tolerance in radians; the same value as OCCT `Precision::Angular()`.
pub const DEFAULT_ANGULAR_TOLERANCE_RAD: f64 = 1.0e-12;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TolerancePolicy {
    linear_mm: f64,
    angular_rad: f64,
}

impl TolerancePolicy {
    pub fn new(linear_mm: f64) -> Result<Self, InvalidTolerance> {
        Self::with_angular(linear_mm, DEFAULT_ANGULAR_TOLERANCE_RAD)
    }

    pub fn with_angular(linear_mm: f64, angular_rad: f64) -> Result<Self, InvalidTolerance> {
        let valid = |value: f64| value.is_finite() && value > 0.0;
        if !valid(linear_mm) || !valid(angular_rad) {
            return Err(InvalidTolerance);
        }
        Ok(Self {
            linear_mm,
            angular_rad,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &'static str {
        TOLERANCE_POLICY_ID
    }

    #[must_use]
    pub const fn linear_mm(&self) -> f64 {
        self.linear_mm
    }

    #[must_use]
    pub const fn angular_rad(&self) -> f64 {
        self.angular_rad
    }

    /// Re-checks a policy that did not come through a constructor (deserialized input).
    pub fn validated(self) -> Result<Self, InvalidTolerance> {
        Self::with_angular(self.linear_mm, self.angular_rad)
    }
}

impl Default for TolerancePolicy {
    fn default() -> Self {
        Self {
            linear_mm: DEFAULT_LINEAR_TOLERANCE_MM,
            angular_rad: DEFAULT_ANGULAR_TOLERANCE_RAD,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTolerance;

impl fmt::Display for InvalidTolerance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("tolerance must be finite and positive")
    }
}

impl std::error::Error for InvalidTolerance {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_matches_the_exact_kernel_and_resolves_at_the_coordinate_limit() {
        let policy = TolerancePolicy::default();
        assert_eq!(policy.linear_mm(), DEFAULT_LINEAR_TOLERANCE_MM);
        assert_eq!(policy.angular_rad(), DEFAULT_ANGULAR_TOLERANCE_RAD);
        assert_eq!(policy.validated(), Ok(policy));
        let spacing_at_limit = MAX_COORDINATE_MM * f64::EPSILON;
        assert!(policy.linear_mm() > 100.0 * spacing_at_limit);
    }

    #[test]
    fn non_positive_or_non_finite_tolerances_are_refused() {
        for bad in [0.0, -1.0e-7, f64::NAN, f64::INFINITY] {
            assert_eq!(TolerancePolicy::new(bad), Err(InvalidTolerance));
            assert_eq!(
                TolerancePolicy::with_angular(1.0e-7, bad),
                Err(InvalidTolerance)
            );
        }
    }
}
