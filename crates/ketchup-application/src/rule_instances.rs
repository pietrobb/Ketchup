//! Reconcile root component instances and their enclosing groups.
use crate::{RuleProgramApplyError, rule_components::transform};
use ketchup_model::document::{CanonicalCommand, CommandBatch, OccurrenceId, Snapshot};
use ketchup_program::{ProgramModel, model::ProgramGroup};
use std::collections::{BTreeMap, BTreeSet};

/// Maps a canonical leaf path to its expanded program name at any assembly depth.
pub fn rule_program_part_name(
    snapshot: &Snapshot,
    path: &ketchup_model::document::InstancePath,
) -> Option<String> {
    use ketchup_model::document::{InstancePathStep, LocalOccurrenceKey};
    snapshot.resolve_instance_path(path).ok()?;
    let root = snapshot.occurrence(path.root_occurrence())?;
    let mut owner = root.definition_id();
    let mut name = root.name();
    let mut names = Vec::new();
    for step in path.steps() {
        if let InstancePathStep::Occurrence(local_id) = step {
            if name != snapshot.definition(owner)?.name() {
                names.push(name);
            }
            let member = snapshot.local_occurrence(LocalOccurrenceKey {
                definition_id: owner,
                local_id: *local_id,
            })?;
            owner = member.definition_id();
            name = member.name();
        }
    }
    names.push(name);
    Some(names.join("/"))
}

pub(crate) fn copied_parts(model: &ProgramModel) -> BTreeSet<String> {
    model
        .instances
        .iter()
        .flat_map(|instance| {
            model
                .components
                .iter()
                .filter(|component| component.name == instance.component)
                .flat_map(move |component| {
                    component
                        .parts
                        .iter()
                        .map(move |part| format!("{}/{}", instance.name, part.name))
                })
        })
        .collect()
}

fn outer_groups(model: &ProgramModel) -> Vec<ProgramGroup> {
    let mut internal = model
        .components
        .iter()
        .flat_map(|component| component.groups.iter().map(|group| group.name.clone()))
        .collect::<BTreeSet<_>>();
    for instance in &model.instances {
        for component in model
            .components
            .iter()
            .filter(|item| item.name == instance.component)
        {
            internal.extend(component.groups.iter().map(|group| {
                if group.name == component.name {
                    instance.name.clone()
                } else {
                    format!("{}/{}", instance.name, group.name)
                }
            }));
        }
    }
    model
        .groups
        .iter()
        .filter(|group| !internal.contains(&group.name))
        .cloned()
        .collect()
}

pub(crate) fn reconcile(
    snapshot: &Snapshot,
    old: &ProgramModel,
    new: &ProgramModel,
    mut commands: Vec<CanonicalCommand>,
) -> Result<CommandBatch, RuleProgramApplyError> {
    let unsupported = || RuleProgramApplyError::IncrementalUnsupported;
    let mut definitions = BTreeMap::new();
    for component in &old.components {
        let definition = snapshot
            .definitions()
            .find(|item| item.name() == component.name)
            .ok_or_else(unsupported)?;
        definitions.insert(component.name.as_str(), definition.id());
    }
    let mut next_id = snapshot
        .occurrences()
        .map(|item| item.id().0)
        .chain(commands.iter().filter_map(|c| match c {
            CanonicalCommand::CreateOccurrence { id, .. } => Some(id.0),
            _ => None,
        }))
        .max()
        .unwrap_or(0);
    let old_direct = crate::rule_assembly_members::direct_model(old);
    let new_direct = crate::rule_assembly_members::direct_model(new);
    for before in old
        .instances
        .iter()
        .filter(|i| crate::rule_assembly_members::owner(&old_direct, &i.name).is_none())
    {
        let occurrence = snapshot
            .occurrences()
            .find(|item| item.name() == before.name)
            .ok_or_else(unsupported)?;
        if occurrence.transform() != transform(before.at_mm, before.rotation)? {
            return Err(unsupported());
        }
        if !new.instances.iter().any(|item| item.name == before.name) {
            commands.push(CanonicalCommand::DeleteOccurrence {
                id: occurrence.id(),
            });
        }
    }
    for after in new
        .instances
        .iter()
        .filter(|i| crate::rule_assembly_members::owner(&new_direct, &i.name).is_none())
    {
        let definition_id = *definitions
            .get(after.component.as_str())
            .ok_or_else(unsupported)?;
        let placement = transform(after.at_mm, after.rotation)?;
        if let Some(before) = old.instances.iter().find(|item| item.name == after.name) {
            let occurrence = snapshot
                .occurrences()
                .find(|item| item.name() == before.name)
                .ok_or_else(unsupported)?;
            if occurrence.definition_id() != definition_id {
                commands.push(CanonicalCommand::RepointOccurrence {
                    id: occurrence.id(),
                    definition_id,
                });
            }
            if occurrence.transform() != placement {
                commands.push(CanonicalCommand::SetOccurrenceTransform {
                    id: occurrence.id(),
                    transform: placement,
                });
            }
        } else {
            next_id = next_id.checked_add(1).ok_or_else(unsupported)?;
            commands.push(CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(next_id),
                definition_id,
                name: after.name.clone(),
                transform: placement,
                parent: None,
                tags: Default::default(),
                visible: true,
            });
        }
    }
    let mut internal_parts = copied_parts(new);
    internal_parts.extend(
        new.components
            .iter()
            .flat_map(|component| component.parts.iter().map(|part| part.name.clone())),
    );
    let members = new
        .parts
        .iter()
        .filter(|part| !internal_parts.contains(&part.name))
        .map(|part| part.name.as_str())
        .chain(
            new.components
                .iter()
                .map(|component| component.name.as_str()),
        )
        .chain(new.instances.iter().map(|instance| instance.name.as_str()))
        .filter(|name| crate::rule_assembly_members::owner(&new_direct, name).is_none());
    crate::rule_groups::reconcile_members(
        snapshot,
        &outer_groups(old),
        &outer_groups(new),
        members,
        CommandBatch::new(commands),
    )
}
