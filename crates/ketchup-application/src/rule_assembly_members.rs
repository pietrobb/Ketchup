//! Project expanded program assemblies onto their directly owned canonical members.
use crate::{RuleProgramApplyError, rule_components::transform};
use ketchup_model::document::{
    CanonicalCommand, DefinitionId, LocalGroupId, LocalOccurrenceId, LocalOccurrenceKey, Snapshot,
    Transform,
};
use ketchup_program::ProgramModel;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn direct_model(model: &ProgramModel) -> ProgramModel {
    let boundaries = model
        .components
        .iter()
        .map(|c| c.name.as_str())
        .chain(model.instances.iter().map(|i| i.name.as_str()))
        .collect::<BTreeSet<_>>();
    let mut direct = model.clone();
    for component in &mut direct.components {
        let mut groups = BTreeSet::new();
        let mut members = BTreeSet::new();
        let mut pending = vec![component.name.clone()];
        while let Some(name) = pending.pop() {
            if !groups.insert(name.clone()) {
                continue;
            }
            if let Some(group) = component.groups.iter().find(|g| g.name == name) {
                for member in &group.members {
                    members.insert(member.clone());
                    if !boundaries.contains(member.as_str())
                        && component.groups.iter().any(|g| &g.name == member)
                    {
                        pending.push(member.clone());
                    }
                }
            }
        }
        component.groups.retain(|g| groups.contains(&g.name));
        component.parts.retain(|p| members.contains(&p.name));
    }
    direct
}

pub(crate) fn owner<'a>(model: &'a ProgramModel, member: &str) -> Option<&'a str> {
    model
        .components
        .iter()
        .find(|component| {
            component
                .groups
                .iter()
                .any(|group| group.members.iter().any(|name| name == member))
        })
        .map(|component| component.name.as_str())
}

fn targets<'a>(
    model: &'a ProgramModel,
    component: &str,
) -> Result<BTreeMap<&'a str, (&'a str, Transform)>, RuleProgramApplyError> {
    let mut targets = BTreeMap::new();
    for child in &model.components {
        if owner(model, &child.name) == Some(component) {
            targets.insert(
                child.name.as_str(),
                (child.name.as_str(), Transform::identity()),
            );
        }
    }
    for instance in &model.instances {
        if owner(model, &instance.name) == Some(component) {
            targets.insert(
                instance.name.as_str(),
                (
                    instance.component.as_str(),
                    transform(instance.at_mm, instance.rotation)?,
                ),
            );
        }
    }
    Ok(targets)
}

pub(crate) fn reconcile(
    snapshot: &Snapshot,
    old: &ProgramModel,
    new: &ProgramModel,
    component: &str,
    parents: &BTreeMap<String, LocalGroupId>,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), RuleProgramApplyError> {
    let unsupported = || RuleProgramApplyError::IncrementalUnsupported;
    let owner = snapshot
        .definitions()
        .find(|d| d.name() == component)
        .ok_or_else(unsupported)?
        .id();
    let before = targets(old, component)?;
    let after = targets(new, component)?;
    let existing = snapshot
        .local_occurrences()
        .filter(|m| m.key().definition_id == owner)
        .map(|m| (m.name(), m))
        .collect::<BTreeMap<_, _>>();
    let mut next_id = existing
        .values()
        .map(|m| m.key().local_id.0)
        .chain(
            snapshot
                .local_groups()
                .filter(|g| g.key().definition_id == owner)
                .map(|g| g.key().local_id.0),
        )
        .chain(commands.iter().filter_map(|c| match c {
            CanonicalCommand::CreateLocalOccurrence { key, .. } if key.definition_id == owner => {
                Some(key.local_id.0)
            }
            CanonicalCommand::CreateLocalGroup { key, .. } if key.definition_id == owner => {
                Some(key.local_id.0)
            }
            _ => None,
        }))
        .max()
        .unwrap_or(0);
    for (name, (_, placement)) in &before {
        let member = existing.get(name).ok_or_else(unsupported)?;
        if member.transform() != *placement {
            return Err(unsupported());
        }
        if !after.contains_key(name) {
            commands.push(CanonicalCommand::DeleteLocalOccurrence { key: member.key() });
        }
    }
    let groups = &new
        .components
        .iter()
        .find(|c| c.name == component)
        .ok_or_else(unsupported)?
        .groups;
    for (name, (definition, placement)) in after {
        let definition_id: DefinitionId = snapshot
            .definitions()
            .find(|d| d.name() == definition)
            .ok_or_else(unsupported)?
            .id();
        let group = groups
            .iter()
            .find(|g| g.members.iter().any(|m| m == name))
            .ok_or_else(unsupported)?;
        let parent = if group.name == component {
            None
        } else {
            Some(*parents.get(&group.name).ok_or_else(unsupported)?)
        };
        if let Some(member) = existing.get(name) {
            if member.definition_id() != definition_id {
                commands.push(CanonicalCommand::RepointLocalOccurrence {
                    key: member.key(),
                    definition_id,
                });
            }
            if member.transform() != placement {
                commands.push(CanonicalCommand::SetLocalOccurrenceTransform {
                    key: member.key(),
                    transform: placement,
                });
            }
            if member.parent() != parent {
                commands.push(CanonicalCommand::SetLocalOccurrenceParent {
                    key: member.key(),
                    parent,
                });
            }
        } else {
            next_id = next_id.checked_add(1).ok_or_else(unsupported)?;
            commands.push(CanonicalCommand::CreateLocalOccurrence {
                key: LocalOccurrenceKey {
                    definition_id: owner,
                    local_id: LocalOccurrenceId(next_id),
                },
                definition_id,
                name: name.to_owned(),
                transform: placement,
                parent,
                tag: None,
                visible: true,
            });
        }
    }
    Ok(())
}
