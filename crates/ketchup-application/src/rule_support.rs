//! Publish support declarations together with the program's geometry and source.
use crate::{RuleProgramApplyError, SessionError, rule_program_part_name};
use ketchup_model::document::{CanonicalCommand, CommandBatch, DocumentStore};
use ketchup_program::ProgramModel;
use std::collections::BTreeSet;

pub(crate) fn append(
    document: &DocumentStore,
    model: &ProgramModel,
    batch: CommandBatch,
) -> Result<CommandBatch, RuleProgramApplyError> {
    let names = model.grounded_parts();
    let mut commands = batch.commands().to_vec();
    let snapshot = document.current();
    if snapshot.floor_z_mm() != model.floor_z_mm {
        commands.push(CanonicalCommand::SetFloorHeight {
            z_mm: model.floor_z_mm,
        });
    }
    if !names.is_empty() || !snapshot.grounded_instances().is_empty() {
        let mut staged = document.fork_for_planning();
        if !batch.commands().is_empty() {
            staged
                .apply_batch(&batch)
                .map_err(SessionError::Canonical)?;
        }
        let snapshot = staged.current();
        let mut paths = BTreeSet::new();
        let mut found = BTreeSet::new();
        for leaf in snapshot.scene_query() {
            if let Some(name) = rule_program_part_name(&snapshot, &leaf.instance_path)
                && names.contains(&name)
            {
                paths.insert(leaf.instance_path);
                found.insert(name);
            }
        }
        if found != names {
            return Err(RuleProgramApplyError::IncrementalUnsupported);
        }
        if &paths != snapshot.grounded_instances() {
            commands.push(CanonicalCommand::SetGroundedInstances { paths });
        }
    }
    Ok(CommandBatch::new(commands))
}
