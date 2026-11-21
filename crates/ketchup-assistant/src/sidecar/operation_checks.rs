//! The schema checks of one operation of a CAD edit program, so a refusal can
//! name the operation it belongs to (R1121ac P3).

use super::*;

impl AssistantCadEditProgram {
    /// How many targets operation `operation_index` acts on and how many
    /// occurrences it generates for each, or why it is refused.
    pub(super) fn validate_operation(
        &self,
        operation_index: usize,
        max_part_cuts: usize,
        named_outputs: &mut BTreeMap<String, AssistantCadProgramFeatureOutput>,
    ) -> Result<(usize, usize), AssistantRequestInvalid> {
        let operation = &self.operations[operation_index];
        let bounded_targets = Self::bounded_operation_targets(operation)?;
        let generated_per_target =
            self.validate_part_operation(operation_index, operation, max_part_cuts)?
                + self.validate_construction_operation(operation_index, operation)?
                + self.validate_feature_operation(operation_index, operation, named_outputs)?
                + self.validate_joint_operation(operation_index, operation)?
                + Self::validate_cam_operation(operation)?
                + Self::validate_metadata_operation(operation)?
                + self.validate_transform_operation(operation_index, operation)?;
        Ok((bounded_targets, generated_per_target))
    }
}
