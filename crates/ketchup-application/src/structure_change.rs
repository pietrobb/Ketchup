//! Plans the `structure` CAD edit operation: one scene-tree change compiled to
//! one canonical command. The document store enforces the invariants (empty
//! group, unassigned tag, unused definition, valid nesting) when the batch is
//! previewed, so a refused change never reaches the document.

use crate::diagnostics::{AssistantPlanningResult, assistant_canonical_rejection};
use ketchup_assistant::sidecar::AssistantStructureChange;
use ketchup_model::document::{
    CanonicalCommand, CanonicalError, ConvertGroupPlan, DefinitionId, GroupId, OccurrenceId,
    Snapshot, TagId, Transform,
};

/// The command for `change`, and the definition and occurrence it creates, if any.
pub(crate) fn plan_structure_change(
    change: &AssistantStructureChange,
    snapshot: &Snapshot,
    next_definition: &mut Option<u64>,
    next_occurrence: &mut Option<u64>,
    target: &str,
) -> AssistantPlanningResult<(CanonicalCommand, Option<(DefinitionId, OccurrenceId)>)> {
    let rejected = |error| assistant_canonical_rejection(error, "structure", target);
    let command = match change {
        AssistantStructureChange::CreateGroup {
            group_id,
            name,
            parent_group_id,
        } => CanonicalCommand::CreateGroup {
            id: GroupId(*group_id),
            name: name.clone(),
            transform: Transform::identity(),
            parent: parent_group_id.map(GroupId),
        },
        AssistantStructureChange::MoveOccurrence {
            occurrence_id,
            group_id,
        } => CanonicalCommand::SetOccurrenceParent {
            id: OccurrenceId(*occurrence_id),
            parent: group_id.map(GroupId),
        },
        AssistantStructureChange::MoveGroup {
            group_id,
            parent_group_id,
        } => CanonicalCommand::SetGroupParent {
            id: GroupId(*group_id),
            parent: parent_group_id.map(GroupId),
        },
        AssistantStructureChange::SetGroupTranslation {
            group_id,
            translation_mm,
        } => {
            let id = GroupId(*group_id);
            let mut matrix = *snapshot
                .group(id)
                .ok_or_else(|| rejected(CanonicalError::GroupNotFound(id)))?
                .transform()
                .matrix();
            [matrix[3], matrix[7], matrix[11]] = *translation_mm;
            CanonicalCommand::SetGroupTransform {
                id,
                transform: Transform::from_matrix(matrix).map_err(rejected)?,
            }
        }
        AssistantStructureChange::DeleteGroup { group_id } => CanonicalCommand::DeleteGroup {
            id: GroupId(*group_id),
        },
        AssistantStructureChange::MakeComponent { group_id, name } => {
            let definition_id =
                DefinitionId(next_definition.ok_or_else(|| rejected(CanonicalError::IdExhausted))?);
            let occurrence_id =
                OccurrenceId(next_occurrence.ok_or_else(|| rejected(CanonicalError::IdExhausted))?);
            *next_definition = definition_id.0.checked_add(1);
            *next_occurrence = occurrence_id.0.checked_add(1);
            return Ok((
                CanonicalCommand::ConvertGroupToComponent(ConvertGroupPlan::new(
                    GroupId(*group_id),
                    definition_id,
                    occurrence_id,
                    name.clone(),
                )),
                Some((definition_id, occurrence_id)),
            ));
        }
        AssistantStructureChange::ReplaceDefinition {
            occurrence_id,
            definition_id,
        } => CanonicalCommand::RepointOccurrence {
            id: OccurrenceId(*occurrence_id),
            definition_id: DefinitionId(*definition_id),
        },
        AssistantStructureChange::RenameDefinition {
            definition_id,
            name,
        } => CanonicalCommand::RenameDefinition {
            id: DefinitionId(*definition_id),
            name: name.clone(),
        },
        AssistantStructureChange::SetOccurrenceVisibility {
            occurrence_id,
            visible,
        } => CanonicalCommand::SetOccurrenceVisibility {
            id: OccurrenceId(*occurrence_id),
            visible: *visible,
        },
        AssistantStructureChange::DeleteTag { tag_id } => {
            CanonicalCommand::DeleteTag { id: TagId(*tag_id) }
        }
        AssistantStructureChange::DeleteDefinition { definition_id } => {
            CanonicalCommand::DeleteDefinition {
                id: DefinitionId(*definition_id),
            }
        }
    };
    Ok((command, None))
}
