//! Reconcile program group membership without replacing part or group identities.
use crate::{RuleProgramApplyError, SessionError};
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DocumentStore, GroupId, Snapshot, Transform,
};
use ketchup_program::{ProgramModel, model::ProgramGroup};
use std::collections::BTreeMap;

pub fn plan_rule_model_batch(
    document: &DocumentStore,
    model: &ProgramModel,
) -> Result<CommandBatch, RuleProgramApplyError> {
    let batch = if !model.components.is_empty() {
        crate::rule_components::build(document, model)?
    } else {
        let parts = crate::planner::plan_rule_part_batch(document, &model.parts)
            .map_err(SessionError::Planning)?;
        reconcile(&document.current(), &ProgramModel::default(), model, parts)?
    };
    let batch = crate::rule_support::append(document, model, batch)?;
    crate::rule_motion::append_pose(document, model, batch)
}

pub(crate) fn reconcile(
    snapshot: &Snapshot,
    old: &ProgramModel,
    new: &ProgramModel,
    parts: CommandBatch,
) -> Result<CommandBatch, RuleProgramApplyError> {
    reconcile_members(
        snapshot,
        &old.groups,
        &new.groups,
        new.parts.iter().map(|part| part.name.as_str()),
        parts,
    )
}

pub(crate) fn reconcile_members<'a>(
    snapshot: &Snapshot,
    old: &[ProgramGroup],
    new: &[ProgramGroup],
    members: impl Iterator<Item = &'a str>,
    parts: CommandBatch,
) -> Result<CommandBatch, RuleProgramApplyError> {
    if old.is_empty() && new.is_empty() {
        return Ok(parts);
    }
    let mut commands = parts.commands().to_vec();
    let mut occurrences = snapshot
        .occurrences()
        .map(|occurrence| {
            (
                occurrence.id(),
                (occurrence.name().to_owned(), occurrence.parent()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for command in parts.commands() {
        match command {
            CanonicalCommand::CreateOccurrence {
                id, name, parent, ..
            } => {
                occurrences.insert(*id, (name.clone(), *parent));
            }
            CanonicalCommand::DeleteOccurrence { id } => {
                occurrences.remove(id);
            }
            _ => {}
        }
    }
    let mut groups = BTreeMap::new();
    for group in old {
        let existing = snapshot
            .groups()
            .find(|item| item.name() == group.name)
            .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
        if existing.transform() != Transform::identity() {
            return Err(RuleProgramApplyError::IncrementalUnsupported);
        }
        groups.insert(group.name.clone(), existing.id());
    }
    let mut next_id = snapshot
        .groups()
        .map(|group| group.id().0)
        .chain(occurrences.keys().map(|id| id.0))
        .max()
        .unwrap_or(0);
    for group in new {
        if groups.contains_key(&group.name) {
            continue;
        }
        if snapshot.groups().any(|item| item.name() == group.name) {
            return Err(RuleProgramApplyError::IncrementalUnsupported);
        }
        next_id = next_id
            .checked_add(1)
            .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
        let id = GroupId(next_id);
        groups.insert(group.name.clone(), id);
        commands.push(CanonicalCommand::CreateGroup {
            id,
            name: group.name.clone(),
            transform: Transform::identity(),
            parent: None,
        });
    }
    let parents = new
        .iter()
        .flat_map(|group| {
            group
                .members
                .iter()
                .map(|member| (member.as_str(), groups[&group.name]))
        })
        .collect::<BTreeMap<_, _>>();
    for member in members {
        let (id, (_, current_parent)) = occurrences
            .iter()
            .find(|(_, (name, _))| name == member)
            .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
        let parent = parents.get(member).copied();
        if *current_parent != parent {
            commands.push(CanonicalCommand::SetOccurrenceParent { id: *id, parent });
        }
    }
    // Detach every removed group's children before deletion, irrespective of declaration order.
    for (name, id) in &groups {
        let parent = parents.get(name.as_str()).copied();
        let current = snapshot.group(*id).and_then(|group| group.parent());
        if current != parent {
            commands.push(CanonicalCommand::SetGroupParent { id: *id, parent });
        }
    }
    for group in old {
        if !new.iter().any(|item| item.name == group.name) {
            commands.push(CanonicalCommand::DeleteGroup {
                id: groups[&group.name],
            });
        }
    }
    Ok(CommandBatch::new(commands))
}
