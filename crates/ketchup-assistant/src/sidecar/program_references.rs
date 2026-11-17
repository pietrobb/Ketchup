//! Which earlier program operations a body feature may name: each reference
//! must point back to an operation that produces the right kind of output.

use super::*;

impl AssistantCadBodyFeature {
    pub(super) fn validate_program_references(
        &self,
        operation_index: usize,
        operations: &[AssistantCadEditOperation],
    ) -> Result<(), AssistantRequestInvalid> {
        let validate_reference = |reference: AssistantCadFeatureReference| match reference {
            AssistantCadFeatureReference::Existing(_) => Ok(()),
            AssistantCadFeatureReference::ProgramOutput(reference) => reference.validate_for(
                operation_index,
                operations,
                AssistantCadProgramFeatureOutput::BodyFeature,
            ),
        };
        match self {
            Self::Boolean {
                target_feature_id,
                tool_feature_id,
                ..
            } => {
                validate_reference(*target_feature_id)?;
                validate_reference(*tool_feature_id)?;
            }
            Self::WeldmentJoint {
                first_member_id,
                second_member_id,
                ..
            } => {
                validate_reference(*first_member_id)?;
                validate_reference(*second_member_id)?;
            }
            Self::Loft {
                sections,
                guide_feature_id,
                rail_feature_id,
                ..
            }
            | Self::SurfaceBody {
                source:
                    AssistantCadSurfaceBodySource::Loft {
                        sections,
                        guide_feature_id,
                        rail_feature_id,
                        ..
                    },
            } => {
                for section in sections {
                    if let AssistantCadFeatureReference::ProgramOutput(reference) =
                        section.profile_feature_id
                    {
                        reference.validate_for(
                            operation_index,
                            operations,
                            AssistantCadProgramFeatureOutput::SketchFeature,
                        )?;
                    }
                }
                for path in guide_feature_id.iter().chain(rail_feature_id) {
                    if let AssistantCadFeatureReference::ProgramOutput(reference) = path {
                        reference.validate_for(
                            operation_index,
                            operations,
                            AssistantCadProgramFeatureOutput::ConstructionFeature,
                        )?;
                    }
                }
            }
            Self::SurfaceBody {
                source:
                    AssistantCadSurfaceBodySource::Planar {
                        profile_feature_id: AssistantCadFeatureReference::ProgramOutput(reference),
                    },
            } => {
                reference.validate_for(
                    operation_index,
                    operations,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                )?;
            }
            Self::SurfaceBody {
                source:
                    AssistantCadSurfaceBodySource::Planar { .. }
                    | AssistantCadSurfaceBodySource::Boundary { .. },
            } => {}
            Self::SurfaceTrim {
                target_feature_id,
                cutter_feature_id,
            }
            | Self::SurfaceBody {
                source:
                    AssistantCadSurfaceBodySource::Blend {
                        first_feature_id: target_feature_id,
                        second_feature_id: cutter_feature_id,
                        ..
                    },
            } => {
                validate_reference(*target_feature_id)?;
                validate_reference(*cutter_feature_id)?;
            }
            Self::SurfaceBody {
                source:
                    AssistantCadSurfaceBodySource::ControlPoint {
                        target_feature_id,
                        keep,
                        ..
                    },
            } => {
                validate_reference(*target_feature_id)?;
                for seam in keep {
                    validate_reference(seam.surface_feature_id)?;
                }
            }
            Self::SurfaceExtend {
                target_feature_id, ..
            }
            | Self::SurfaceThicken {
                target_feature_id, ..
            } => validate_reference(*target_feature_id)?,
            Self::SurfaceKnit {
                surface_feature_ids,
                ..
            } => {
                for reference in surface_feature_ids {
                    validate_reference(*reference)?;
                }
            }
            Self::Pocket {
                target_feature_id,
                profile_feature_id,
                ..
            } => {
                validate_reference(*target_feature_id)?;
                profile_feature_id.validate_output(
                    operation_index,
                    operations,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                )?;
            }
            _ => {}
        }
        Ok(())
    }
}
