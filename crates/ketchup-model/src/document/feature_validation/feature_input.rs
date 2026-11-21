//! Why the input of a surface or loft feature is refused: the field it names and
//! how to repair it (R1120ac-EXA-901). Before, a weight of zero, a seam kept
//! twice or a Blend of a surface with itself answered "feature 620 has invalid
//! definition ownership" for the target surface, a bad Boundary answered the
//! Sweep rule, and every guide or rail fault the section rule of a Loft.

use super::*;
use ketchup_tolerance::limits;
#[cfg(test)]
#[path = "feature_input_tests.rs"]
mod tests;

/// The refused field of a feature's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureInputField {
    FromMm,
    OffsetMm,
    Weight,
    Keep,
    /// The seam at this index of `keep`.
    KeepSeam(usize),
    Second,
    Cutter,
    Segments,
    Inner,
    Sections,
    /// The profile of the section at this index.
    SectionProfile(usize),
    /// The elevation of the section at this index.
    SectionElevation(usize),
    Guide,
    Rail,
    Continuity,
}

impl fmt::Display for FeatureInputField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FromMm => formatter.write_str("from_mm"),
            Self::OffsetMm => formatter.write_str("offset_mm"),
            Self::Weight => formatter.write_str("weight"),
            Self::Keep => formatter.write_str("keep"),
            Self::KeepSeam(index) => write!(formatter, "keep[{index}]"),
            Self::Second => formatter.write_str("second"),
            Self::Cutter => formatter.write_str("cutter"),
            Self::Segments => formatter.write_str("segments"),
            Self::Inner => formatter.write_str("inner"),
            Self::Sections => formatter.write_str("sections"),
            Self::SectionProfile(index) => write!(formatter, "sections[{index}].profile"),
            Self::SectionElevation(index) => write!(formatter, "sections[{index}].elevation_mm"),
            Self::Guide => formatter.write_str("guide"),
            Self::Rail => formatter.write_str("rail"),
            Self::Continuity => formatter.write_str("continuity"),
        }
    }
}

/// A refused feature input: the feature once known, the field, and the rule it
/// breaks with its repair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureInputError {
    /// The refused feature: `None` while only its kind is checked, set by the
    /// command or document check that knows it (R1121ac P3).
    pub feature: Option<FeatureId>,
    pub field: FeatureInputField,
    pub rule: &'static str,
}

impl fmt::Display for FeatureInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.feature {
            Some(feature) => write!(
                formatter,
                "feature {} input {} is invalid: {}",
                feature.0, self.field, self.rule
            ),
            None => write!(
                formatter,
                "feature input {} is invalid: {}",
                self.field, self.rule
            ),
        }
    }
}

impl CanonicalError {
    /// This error with `feature` named as the refused feature, when it is a
    /// feature input refused before the feature was known.
    #[must_use]
    pub(in crate::document) fn of_feature(self, feature: FeatureId) -> Self {
        match self {
            Self::InvalidFeatureInput(error @ FeatureInputError { feature: None, .. }) => {
                Self::InvalidFeatureInput(FeatureInputError {
                    feature: Some(feature),
                    ..error
                })
            }
            other => other,
        }
    }
}

/// Ok when `holds`, else `field` refused for breaking `rule`.
const fn check(
    holds: bool,
    field: FeatureInputField,
    rule: &'static str,
) -> Result<(), CanonicalError> {
    if holds {
        Ok(())
    } else {
        Err(CanonicalError::InvalidFeatureInput(FeatureInputError {
            feature: None,
            field,
            rule,
        }))
    }
}

const EARLIER_PATH: &str = "it must be a spatial path of the same part, placed before this loft";
const TANGENT_AT_MOST: &str =
    "a loft along a guide curve is at most tangent-continuous; drop the guide or choose tangent";

/// A planar, blend, control-point or boundary surface body; [`loft`] checks
/// the sections of a lofted one.
pub(in crate::document) fn surface_body(
    spec: &SurfaceBodySpec,
    tolerance_mm: f64,
) -> Result<(), CanonicalError> {
    match spec {
        SurfaceBodySpec::Blend { first, second, .. } => check(
            first != second,
            FeatureInputField::Second,
            "a blend spans two different surfaces",
        ),
        SurfaceBodySpec::Boundary { segments, inner } => {
            check(
                is_valid_boundary_loop(segments, tolerance_mm),
                FeatureInputField::Segments,
                "the boundary must be one closed loop of 2 to 64 bounded lines, arcs or cubic \
                 curves, each ending where the next starts",
            )?;
            check(
                are_valid_network_curves(inner, tolerance_mm),
                FeatureInputField::Inner,
                "the inner curves must be at most 64 bounded lines, arcs or cubic curves",
            )
        }
        SurfaceBodySpec::ControlPoint {
            target,
            from_mm,
            offset_mm,
            weight,
            keep,
            ..
        } => {
            let bounded = |values: &[f64; 3]| {
                values
                    .iter()
                    .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
            };
            check(
                bounded(from_mm),
                FeatureInputField::FromMm,
                "each value must be finite and within the coordinate limit; read the point \
                 with inspect surface_control_net",
            )?;
            check(
                bounded(offset_mm),
                FeatureInputField::OffsetMm,
                "each value must be finite and within the coordinate limit",
            )?;
            check(
                weight.is_none_or(|weight| {
                    (1.0 / limits::NURBS_WEIGHT_SPAN..=limits::NURBS_WEIGHT_SPAN).contains(&weight)
                }),
                FeatureInputField::Weight,
                "a weight must lie between 0.000001 and 1000000; 1 leaves the point as plain \
                 as its neighbours",
            )?;
            check(
                weight.is_some() || offset_mm.iter().any(|value| *value != 0.0),
                FeatureInputField::OffsetMm,
                "the move changes nothing: give a nonzero offset or a weight",
            )?;
            check(
                keep.len() <= limits::FEATURE_REFERENCES,
                FeatureInputField::Keep,
                "it keeps more seams than a feature may reference",
            )?;
            for (index, seam) in keep.iter().enumerate() {
                check(
                    seam.surface != *target,
                    FeatureInputField::KeepSeam(index),
                    "the surface that moves cannot keep a seam with itself; name a neighbouring \
                     surface",
                )?;
                check(
                    keep[..index]
                        .iter()
                        .all(|other| other.surface != seam.surface),
                    FeatureInputField::KeepSeam(index),
                    "this surface is already kept; name each neighbour once",
                )?;
            }
            Ok(())
        }
        SurfaceBodySpec::Planar { .. } | SurfaceBodySpec::Loft { .. } => Ok(()),
    }
}

/// A surface trimmed by another surface.
pub(in crate::document) fn surface_trim(
    target: FeatureId,
    cutter: FeatureId,
) -> Result<(), CanonicalError> {
    check(
        target != cutter,
        FeatureInputField::Cutter,
        "a surface cannot trim itself; name another surface as the cutter",
    )
}

/// The sections, guide and rail of a loft, before the document resolves them.
pub(in crate::document) fn loft(
    sections: &[LoftSection],
    guide: Option<FeatureId>,
    continuity: LoftContinuity,
    rail: Option<FeatureId>,
) -> Result<(), CanonicalError> {
    check(
        (2..=limits::LOFT_SECTIONS).contains(&sections.len()),
        FeatureInputField::Sections,
        "a loft needs 2 to 16 sections",
    )?;
    check(
        guide.is_none() || continuity != LoftContinuity::Curvature,
        FeatureInputField::Continuity,
        TANGENT_AT_MOST,
    )?;
    if rail.is_some() {
        check(
            guide.is_some(),
            FeatureInputField::Rail,
            "a rail needs a guide: the rail is the second guide curve",
        )?;
        check(
            rail != guide,
            FeatureInputField::Rail,
            "the rail must be a different spatial path than the guide",
        )?;
    }
    for (index, section) in sections.iter().enumerate() {
        check(
            section.elevation_mm.is_finite() && section.elevation_mm.abs() <= MAX_COORDINATE_MM,
            FeatureInputField::SectionElevation(index),
            "it must be finite and within the coordinate limit",
        )?;
        check(
            index == 0 || sections[index - 1].elevation_mm < section.elevation_mm,
            FeatureInputField::SectionElevation(index),
            "sections must rise: each elevation greater than the one before",
        )?;
        check(
            sections[..index]
                .iter()
                .all(|other| other.profile != section.profile),
            FeatureInputField::SectionProfile(index),
            "this profile is already a section; each section needs its own profile",
        )?;
    }
    Ok(())
}

/// Section `index` of a loft resolves to a profile it can loft: an earlier
/// feature of the same part with one closed region or one closed spline.
pub(in crate::document) fn loft_profile(index: usize, valid: bool) -> Result<(), CanonicalError> {
    check(
        valid,
        FeatureInputField::SectionProfile(index),
        "it must be a sketch with one closed region or a closed spline of the same part, \
         placed before this loft",
    )
}

/// The guide or rail `field` of a loft resolves to an earlier spatial path of
/// the same part, and a loft along it is at most tangent-continuous.
pub(in crate::document) fn loft_path(
    field: FeatureInputField,
    continuity: LoftContinuity,
    earlier_path: bool,
) -> Result<(), CanonicalError> {
    check(
        continuity != LoftContinuity::Curvature,
        FeatureInputField::Continuity,
        TANGENT_AT_MOST,
    )?;
    check(earlier_path, field, EARLIER_PATH)
}
