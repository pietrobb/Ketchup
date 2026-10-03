//! Publish program colors without rebuilding geometry, including shared leaves.
use crate::RuleProgramApplyError;
use ketchup_model::document::{
    CanonicalCommand, InstancePath, InstancePathStep, LocalOccurrenceKey, Snapshot,
};
use ketchup_program::ProgramModel;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn needed(snapshot: &Snapshot, model: &ProgramModel) -> bool {
    model.parts.iter().any(|part| part.color.is_some())
        || snapshot.occurrences().any(|part| part.color().is_some())
        || snapshot
            .local_occurrences()
            .any(|part| part.color().is_some())
}

pub(crate) fn append(
    snapshot: &Snapshot,
    model: &ProgramModel,
    names: &BTreeMap<String, InstancePath>,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), RuleProgramApplyError> {
    let mut seen = BTreeSet::new();
    for part in &model.parts {
        let path = names
            .get(&part.name)
            .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
        let root = snapshot
            .occurrence(path.root_occurrence())
            .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
        let mut owner = root.definition_id();
        let mut local = None;
        for step in path.steps() {
            if let InstancePathStep::Occurrence(id) = step {
                let member = snapshot
                    .local_occurrence(LocalOccurrenceKey {
                        definition_id: owner,
                        local_id: *id,
                    })
                    .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
                owner = member.definition_id();
                local = Some(member);
            }
        }
        if let Some(member) = local {
            if member.color() != part.color && seen.insert(member.key()) {
                commands.push(CanonicalCommand::SetLocalOccurrenceColor {
                    key: member.key(),
                    color: part.color,
                });
            }
        } else if root.color() != part.color {
            commands.push(CanonicalCommand::SetOccurrenceColor {
                id: root.id(),
                color: part.color,
            });
        }
    }
    Ok(())
}
