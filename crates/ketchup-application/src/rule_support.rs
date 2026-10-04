//! Publish support and physical contact declarations with the program's geometry.
use crate::{RuleProgramApplyError, SessionError, rule_program_part_name};
use ketchup_model::document::{CanonicalCommand, CommandBatch, ContactJoint, DocumentStore};
use ketchup_program::ProgramModel;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn append(
    document: &DocumentStore,
    model: &ProgramModel,
    old: Option<&ProgramModel>,
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
    if !names.is_empty()
        || !snapshot.grounded_instances().is_empty()
        || !model.joints.is_empty()
        || !snapshot.contact_joints().is_empty()
        || crate::rule_appearance::needed(&snapshot, model)
        || crate::rule_metadata::needed(model)
        || old.is_some_and(crate::rule_metadata::needed)
        || crate::rule_tags::needed(model)
        || old.is_some_and(crate::rule_tags::needed)
    {
        let mut staged = document.fork_for_planning();
        if !batch.commands().is_empty() {
            staged
                .apply_batch(&batch)
                .map_err(SessionError::Canonical)?;
        }
        let snapshot = staged.current();
        let by_name = snapshot
            .scene_query()
            .into_iter()
            .filter_map(|leaf| {
                rule_program_part_name(&snapshot, &leaf.instance_path)
                    .map(|name| (name, leaf.instance_path))
            })
            .collect::<BTreeMap<_, _>>();
        crate::rule_appearance::append(&snapshot, model, &by_name, &mut commands)?;
        crate::rule_metadata::append(
            &document.current(),
            &snapshot,
            old,
            model,
            &by_name,
            &mut commands,
        )?;
        crate::rule_tags::append(&snapshot, old, model, &by_name, &mut commands)?;
        let paths = names
            .iter()
            .map(|name| {
                by_name
                    .get(name)
                    .cloned()
                    .ok_or(RuleProgramApplyError::IncrementalUnsupported)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if &paths != snapshot.grounded_instances() {
            commands.push(CanonicalCommand::SetGroundedInstances { paths });
        }
        let joints = model
            .joints
            .iter()
            .map(|joint| {
                let parts = [&joint.parts[0], &joint.parts[1]].map(|name| {
                    by_name
                        .get(name)
                        .cloned()
                        .ok_or(RuleProgramApplyError::IncrementalUnsupported)
                });
                let [a, b] = parts;
                Ok(ContactJoint {
                    name: joint.name.clone(),
                    parts: [a?, b?],
                    max_gap_mm: joint.max_gap_mm,
                })
            })
            .collect::<Result<Vec<_>, RuleProgramApplyError>>()?;
        if joints != snapshot.contact_joints() {
            commands.push(CanonicalCommand::SetContactJoints { joints });
        }
    }
    Ok(CommandBatch::new(commands))
}
