//! Validation of one assistant change to the group and component structure.

use super::{AssistantStructureChange, assistant_cad_vector_is_bounded};
use crate::request_invalid::AssistantRequestInvalid;
use ketchup_tolerance::limits;

impl AssistantStructureChange {
    pub(super) fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        let name_is_valid = |name: &str| {
            !name.trim().is_empty()
                && name.len() <= limits::NAME_BYTES
                && !name.chars().any(char::is_control)
        };
        let valid = match self {
            Self::CreateGroup {
                group_id,
                name,
                parent_group_id,
            } => *group_id != 0 && name_is_valid(name) && *parent_group_id != Some(0),
            Self::MoveOccurrence {
                occurrence_id,
                group_id,
            } => *occurrence_id != 0 && *group_id != Some(0),
            Self::MoveGroup {
                group_id,
                parent_group_id,
            } => {
                *group_id != 0 && *parent_group_id != Some(0) && *parent_group_id != Some(*group_id)
            }
            Self::SetGroupTranslation {
                group_id,
                translation_mm,
            } => *group_id != 0 && assistant_cad_vector_is_bounded(*translation_mm),
            Self::MakeComponent { group_id, name } => *group_id != 0 && name_is_valid(name),
            Self::ReplaceDefinition {
                occurrence_id,
                definition_id,
            } => *occurrence_id != 0 && *definition_id != 0,
            Self::RenameDefinition {
                definition_id,
                name,
            } => *definition_id != 0 && name_is_valid(name),
            Self::SetOccurrenceVisibility { occurrence_id, .. } => *occurrence_id != 0,
            Self::DeleteGroup { group_id } => *group_id != 0,
            Self::DeleteTag { tag_id } => *tag_id != 0,
            Self::DeleteDefinition { definition_id } => *definition_id != 0,
        };
        if valid {
            Ok(())
        } else {
            Err(AssistantRequestInvalid::invalid("CAD structure change"))
        }
    }
}
