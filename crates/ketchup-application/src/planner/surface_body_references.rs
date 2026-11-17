//! Program-output references inside a surface body source.

use super::{
    AssistantCadFeatureReference, AssistantCadProgramFeatureOutput, AssistantCadSurfaceBodySource,
    AssistantPlanningResult, StagedPlanningContext,
};
use ketchup_assistant::sidecar::AssistantCadLoftSection;

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
            rail_feature_id,
            ..
        } => resolve_loft(
            sections,
            [guide_feature_id, rail_feature_id],
            staged_planning,
            operation,
        )?,
    }
    Ok(())
}

/// Rewrites the program-output references of a Loft: its section profiles and
/// its guide and rail paths.
pub(super) fn resolve_loft(
    sections: &mut [AssistantCadLoftSection],
    paths: [&mut Option<AssistantCadFeatureReference>; 2],
    staged_planning: &StagedPlanningContext,
    operation: &str,
) -> AssistantPlanningResult<()> {
    for section in sections {
        section.profile_feature_id = staged_planning
            .resolve_program_feature(
                section.profile_feature_id,
                AssistantCadProgramFeatureOutput::SketchFeature,
                operation,
            )?
            .into();
    }
    for path in paths.into_iter().flatten() {
        *path = staged_planning
            .resolve_program_feature(
                *path,
                AssistantCadProgramFeatureOutput::ConstructionFeature,
                operation,
            )?
            .into();
    }
    Ok(())
}
