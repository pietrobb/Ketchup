//! The Loft requests a document can hold.

use super::{AssistantCadFeatureReference, AssistantCadLoftContinuity, AssistantCadLoftSection};
use ketchup_model::tolerance::MAX_COORDINATE_MM;
use ketchup_tolerance::limits;
use std::collections::BTreeSet;

/// A Loft request a document can hold: 2 to the most sections at rising
/// elevations, each a different valid profile; a curvature-continuous Loft has
/// no guide, and a rail needs a guide other than itself.
pub(super) fn valid(
    sections: &[AssistantCadLoftSection],
    guide: Option<AssistantCadFeatureReference>,
    rail: Option<AssistantCadFeatureReference>,
    continuity: AssistantCadLoftContinuity,
) -> bool {
    (2..=limits::LOFT_SECTIONS).contains(&sections.len())
        && guide
            .iter()
            .chain(&rail)
            .all(|path| path.validate().is_ok())
        && !(guide.is_some() && continuity == AssistantCadLoftContinuity::Curvature)
        && (rail.is_none() || (guide.is_some() && rail != guide))
        && sections.iter().all(|section| {
            section.profile_feature_id.validate().is_ok()
                && section.elevation_mm.is_finite()
                && section.elevation_mm.abs() <= MAX_COORDINATE_MM
        })
        && sections
            .iter()
            .map(|section| section.profile_feature_id)
            .collect::<BTreeSet<_>>()
            .len()
            == sections.len()
        && sections
            .windows(2)
            .all(|pair| pair[0].elevation_mm < pair[1].elevation_mm)
}
