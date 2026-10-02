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

pub mod limits;

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

/// Numeric guard, not a model tolerance: the relative rounding allowed for a value
/// computed in `f64`. Compare with `ROUNDING * magnitude`, the magnitude being the size of
/// the operands (at least 1). It keeps divisions away from zero and absorbs rounding of
/// computed angles, areas and cross products.
pub const ROUNDING: f64 = 1.0e-9;

/// Numeric guard for values that went through several computed steps: chained transforms,
/// iterative solving, rank decisions on a solved Jacobian. Their error accumulates, so they
/// agree to a looser bound than a single rounding.
pub const ACCUMULATED_ROUNDING: f64 = 1.0e-8;

/// Agreement allowed where a value is approximated rather than computed exactly: areas,
/// volumes and bounding boxes the exact kernel measures, and curves flattened into lines.
/// Absolute (mm, mm², mm³) for magnitudes up to 1; scale it by larger magnitudes.
pub const APPROXIMATION: f64 = 1.0e-6;

/// Chord error for material face boundaries used by planar contact queries, in mm.
pub const BOUNDARY_CHORD_MM: f64 = 0.001;
/// Smallest angular sampling step for a material boundary, in radians.
pub const BOUNDARY_MIN_ANGLE_RAD: f64 = 0.001;
/// Fraction of an edge interval used to sample either side of a Boolean boundary.
pub const BOUNDARY_PROBE_FRACTION: f64 = 1.0e-4;

/// Numeric guard: a scaled quantity at most this large counts as zero. It marks a singular
/// determinant or pivot, a degenerate area, and a residual that has converged relative to
/// where it started. Such quantities are products of several values, so the bound is
/// tighter than [`ROUNDING`].
pub const NEGLIGIBLE: f64 = 1.0e-12;

/// Relative step of a finite-difference derivative in the iterative solvers. Central
/// differences at this step leave truncation (step²) and cancellation (rounding / step)
/// errors far below [`APPROXIMATION`].
pub const FINITE_DIFFERENCE_STEP: f64 = 1.0e-6;

/// Starting Levenberg–Marquardt damping of the sketch solver, relative to unit-scaled
/// equations; the solver adapts it every iteration.
pub const INITIAL_DAMPING: f64 = 1.0e-6;

/// Numeric guard for screen-space `f32` values in pixels: a projected length or a signed
/// area at most this large is degenerate on screen.
pub const SCREEN_ROUNDING_PX: f32 = 1.0e-4;

/// Deserializing re-checks the values, so a stored policy is valid like a constructed one.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "StoredTolerancePolicy")]
pub struct TolerancePolicy {
    linear_mm: f64,
    angular_rad: f64,
}

// Values are validated finite numbers, never NaN, so equality is total.
impl Eq for TolerancePolicy {}

#[derive(serde::Deserialize)]
struct StoredTolerancePolicy {
    linear_mm: f64,
    angular_rad: f64,
}

impl TryFrom<StoredTolerancePolicy> for TolerancePolicy {
    type Error = InvalidTolerance;

    fn try_from(stored: StoredTolerancePolicy) -> Result<Self, Self::Error> {
        Self::with_angular(stored.linear_mm, stored.angular_rad)
    }
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

    /// Whether this is the default policy; a document stores only a non-default one, so
    /// documents on the default keep their saved bytes and digest.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
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
