//! Program-output references inside a surface body source.

use super::{
    AssistantCadProgramFeatureOutput, AssistantCadSurfaceBodySource, AssistantPlanningResult,
    StagedPlanningContext,
};

/// Rewrites every earlier-program-output reference in `source` to the host
/// feature it produced.
pub(super) fn resolve(
    source: &mut AssistantCadSurfaceBodySource,
    staged_planning: &StagedPlanningContext,
    operation: &str,
) -> AssistantPlanningResult<()> {
    match source {
        AssistantCadSurfaceBodySource::Planar { profile_feature_id } => {
            *profile_feature_id = staged_planning
                .resolve_program_feature(
                    *profile_feature_id,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                    operation,
                )?
                .into();
        }
        AssistantCadSurfaceBodySource::Boundary { .. } => {}
        AssistantCadSurfaceBodySource::Loft {
            sections,
            guide_feature_id,
            ..
        } => {
            for section in sections {
                section.profile_feature_id = staged_planning
                    .resolve_program_feature(
                        section.profile_feature_id,
                        AssistantCadProgramFeatureOutput::SketchFeature,
                        operation,
                    )?
                    .into();
            }
            if let Some(guide) = guide_feature_id {
                *guide = staged_planning
                    .resolve_program_feature(
                        *guide,
                        AssistantCadProgramFeatureOutput::ConstructionFeature,
                        operation,
                    )?
                    .into();
            }
        }
    }
    Ok(())
}
