//! The Loft requests a document can hold.

use super::feature_checks::{Checked, at_most, reference, rule};
use super::{AssistantCadFeatureReference, AssistantCadLoftContinuity, AssistantCadLoftSection};
use ketchup_model::tolerance::MAX_COORDINATE_MM;
use ketchup_tolerance::limits;

/// A Loft request a document can hold: 2 to the most sections at rising
/// elevations, each a different valid profile; a curvature-continuous Loft has
/// no guide, and a rail needs a guide other than itself. A refusal names the
/// field and the rule (R1120ac-EXA-902).
pub(super) fn check(
    sections: &[AssistantCadLoftSection],
    guide: Option<AssistantCadFeatureReference>,
    rail: Option<AssistantCadFeatureReference>,
    continuity: AssistantCadLoftContinuity,
) -> Checked {
    rule(
        sections.len() >= 2,
        "sections",
        "a loft needs at least 2 sections",
    )?;
    at_most(sections.len(), limits::LOFT_SECTIONS, "sections")?;
    if let Some(guide) = guide {
        reference(guide, "guide_feature_id")?;
        rule(
            continuity != AssistantCadLoftContinuity::Curvature,
            "continuity",
            "a loft along a guide curve is at most tangent-continuous; drop the guide or choose tangent",
        )?;
    }
    if let Some(rail) = rail {
        reference(rail, "rail_feature_id")?;
        rule(
            guide.is_some(),
            "rail_feature_id",
            "a rail needs a guide_feature_id: the rail is the second guide curve",
        )?;
        rule(
            Some(rail) != guide,
            "rail_feature_id",
            "the rail must be a different spatial path than the guide",
        )?;
    }
    for (index, section) in sections.iter().enumerate() {
        reference(
            section.profile_feature_id,
            &format!("sections[{index}].profile_feature_id"),
        )?;
        rule(
            sections[..index]
                .iter()
                .all(|other| other.profile_feature_id != section.profile_feature_id),
            format!("sections[{index}].profile_feature_id"),
            "this profile is already a section; each section needs its own profile",
        )?;
        rule(
            section.elevation_mm.is_finite() && section.elevation_mm.abs() <= MAX_COORDINATE_MM,
            format!("sections[{index}].elevation_mm"),
            "it must be finite and within the coordinate limit",
        )?;
        rule(
            index == 0 || sections[index - 1].elevation_mm < section.elevation_mm,
            format!("sections[{index}].elevation_mm"),
            "sections must rise: each elevation greater than the one before",
        )?;
    }
    Ok(())
}
