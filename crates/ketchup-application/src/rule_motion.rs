//! Reconcile reference geometry, then publish assembly poses in the same batch.
use crate::{RuleProgramApplyError, SessionError};
use ketchup_model::document::{CanonicalCommand, CommandBatch, DocumentStore, Snapshot, Transform};
use ketchup_program::ProgramModel;

pub(crate) fn commands(
    snapshot: &Snapshot,
    reference: &ProgramModel,
    posed: bool,
) -> Result<Vec<CanonicalCommand>, RuleProgramApplyError> {
    let mut commands = Vec::new();
    if reference.motions.is_empty() {
        return Ok(commands);
    }
    let direct = crate::rule_assembly_members::direct_model(reference);
    for motion in reference.motions.iter().filter(|m| !m.instanced) {
        let target = &motion.endpoints[1];
        let delta = if posed {
            motion
                .transform()
                .ok_or(RuleProgramApplyError::IncrementalUnsupported)?
        } else {
            Transform::identity()
        };
        let base = if let Some(part) = reference.part(target) {
            crate::rule_components::transform(part.at_mm, part.rotation)?
        } else if let Some(instance) = reference.instances.iter().find(|i| &i.name == target) {
            crate::rule_components::transform(instance.at_mm, instance.rotation)?
        } else {
            Transform::identity()
        };
        let transform = delta.compose(base);
        if let Some(owner) = crate::rule_assembly_members::owner(&direct, target) {
            let definition = snapshot
                .definitions()
                .find(|d| d.name() == owner)
                .ok_or(RuleProgramApplyError::IncrementalUnsupported)?
                .id();
            if let Some(group) = snapshot
                .local_groups()
                .find(|g| g.key().definition_id == definition && g.name() == target)
            {
                if group.transform() != transform {
                    commands.push(CanonicalCommand::SetLocalGroupTransform {
                        key: group.key(),
                        transform,
                    });
                }
            } else {
                let member = snapshot
                    .local_occurrences()
                    .find(|m| m.key().definition_id == definition && m.name() == target)
                    .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
                if member.transform() != transform {
                    commands.push(CanonicalCommand::SetLocalOccurrenceTransform {
                        key: member.key(),
                        transform,
                    });
                }
            }
        } else if let Some(group) = snapshot.groups().find(|g| g.name() == target) {
            if group.transform() != transform {
                commands.push(CanonicalCommand::SetGroupTransform {
                    id: group.id(),
                    transform,
                });
            }
        } else {
            let occurrence = snapshot
                .occurrences()
                .find(|o| o.name() == target)
                .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
            if occurrence.transform() != transform {
                commands.push(CanonicalCommand::SetOccurrenceTransform {
                    id: occurrence.id(),
                    transform,
                });
            }
        }
    }
    Ok(commands)
}

pub(crate) fn append_pose(
    document: &DocumentStore,
    reference: &ProgramModel,
    batch: CommandBatch,
) -> Result<CommandBatch, RuleProgramApplyError> {
    if reference.motions.is_empty() {
        return Ok(batch);
    }
    let mut staged = document.fork_for_planning();
    if !batch.commands().is_empty() {
        staged
            .apply_batch(&batch)
            .map_err(SessionError::Canonical)?;
    }
    let mut all = batch.commands().to_vec();
    all.extend(commands(&staged.current(), reference, true)?);
    Ok(CommandBatch::new(all))
}
