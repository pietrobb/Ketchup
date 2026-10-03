//! Shared assembly definitions for program components.
use crate::{RuleProgramApplyError, SessionError};
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, ConvertGroupPlan, DefinitionId, DocumentStore, GroupId,
    OccurrenceId, Transform,
};
use ketchup_program::{ProgramModel, frame};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn transform(
    at: [f64; 3],
    rotation: frame::Mat3,
) -> Result<Transform, RuleProgramApplyError> {
    let mut matrix = [0.0; 16];
    for row in 0..3 {
        matrix[row * 4..row * 4 + 3].copy_from_slice(&rotation[row]);
        matrix[row * 4 + 3] = at[row];
    }
    matrix[15] = 1.0;
    Transform::from_matrix(matrix).map_err(|error| SessionError::Canonical(error).into())
}

pub(crate) fn build(
    document: &DocumentStore,
    model: &ProgramModel,
) -> Result<CommandBatch, RuleProgramApplyError> {
    let copied = model
        .instances
        .iter()
        .flat_map(|instance| {
            let component = model
                .components
                .iter()
                .find(|item| item.name == instance.component);
            component.into_iter().flat_map(move |component| {
                component
                    .parts
                    .iter()
                    .map(|part| format!("{}/{}", instance.name, part.name))
                    .chain(component.groups.iter().map(|group| {
                        if group.name == component.name {
                            instance.name.clone()
                        } else {
                            format!("{}/{}", instance.name, group.name)
                        }
                    }))
            })
        })
        .collect::<BTreeSet<_>>();
    let mut originals = model.clone();
    originals.parts.retain(|part| !copied.contains(&part.name));
    originals
        .groups
        .retain(|group| !copied.contains(&group.name));
    let parts = crate::planner::plan_rule_part_batch(document, &originals.parts)
        .map_err(SessionError::Planning)?;
    let batch = crate::rule_groups::reconcile(
        &document.current(),
        &ProgramModel::default(),
        &originals,
        parts,
    )?;
    let mut commands = batch.commands().to_vec();
    let snapshot = document.current();
    let mut next_definition = snapshot
        .definitions()
        .map(|item| item.id().0)
        .max()
        .unwrap_or(0);
    let mut next_occurrence = snapshot
        .occurrences()
        .map(|item| item.id().0)
        .max()
        .unwrap_or(0);
    let mut groups = BTreeMap::<String, GroupId>::new();
    for command in &commands {
        match command {
            CanonicalCommand::CreateDefinition { id, .. } => {
                next_definition = next_definition.max(id.0)
            }
            CanonicalCommand::CreateOccurrence { id, .. } => {
                next_occurrence = next_occurrence.max(id.0)
            }
            CanonicalCommand::CreateGroup { id, name, .. } => {
                groups.insert(name.clone(), *id);
                next_occurrence = next_occurrence.max(id.0);
            }
            _ => {}
        }
    }
    for component in &model.components {
        next_definition = next_definition
            .checked_add(1)
            .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
        next_occurrence = next_occurrence
            .checked_add(1)
            .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
        let definition = DefinitionId(next_definition);

        commands.push(CanonicalCommand::ConvertGroupToComponent(
            ConvertGroupPlan::new(
                groups[&component.name],
                definition,
                OccurrenceId(next_occurrence),
                component.name.clone(),
            ),
        ));
        // Publish child instances before converting any enclosing component group.
        for instance in model
            .instances
            .iter()
            .filter(|item| item.component == component.name)
        {
            next_occurrence = next_occurrence
                .checked_add(1)
                .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
            let parent = originals
                .groups
                .iter()
                .find(|group| group.members.contains(&instance.name))
                .map(|group| groups[&group.name]);
            commands.push(CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(next_occurrence),
                definition_id: definition,
                name: instance.name.clone(),
                transform: transform(instance.at_mm, instance.rotation)?,
                parent,
                tag: None,
                visible: true,
            });
        }
    }
    Ok(CommandBatch::new(commands))
}

fn reconcile_local_groups(
    snapshot: &ketchup_model::document::Snapshot,
    owner: DefinitionId,
    after: &ketchup_program::model::ProgramComponent,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<BTreeMap<String, ketchup_model::document::LocalGroupId>, RuleProgramApplyError> {
    use ketchup_model::document::{LocalGroupId, LocalGroupKey};
    let unsupported = || RuleProgramApplyError::IncrementalUnsupported;
    let existing = snapshot
        .local_groups()
        .filter(|group| group.key().definition_id == owner)
        .map(|group| (group.name().to_owned(), group))
        .collect::<BTreeMap<_, _>>();
    let mut next_id = snapshot
        .local_occurrences()
        .filter(|member| member.key().definition_id == owner)
        .map(|member| member.key().local_id.0)
        .chain(existing.values().map(|group| group.key().local_id.0))
        .max()
        .unwrap_or(0);
    let mut ids = BTreeMap::new();
    for group in after.groups.iter().filter(|group| group.name != after.name) {
        let id = if let Some(before) = existing.get(&group.name) {
            if before.transform() != Transform::identity() {
                return Err(unsupported());
            }
            before.key().local_id
        } else {
            next_id = next_id.checked_add(1).ok_or_else(unsupported)?;
            LocalGroupId(next_id)
        };
        ids.insert(group.name.clone(), id);
    }
    for group in after.groups.iter().filter(|group| group.name != after.name) {
        let parent = after
            .groups
            .iter()
            .find(|parent| parent.members.contains(&group.name))
            .ok_or_else(unsupported)?;
        let parent = if parent.name == after.name {
            None
        } else {
            Some(*ids.get(&parent.name).ok_or_else(unsupported)?)
        };
        if let Some(before) = existing.get(&group.name) {
            if before.parent() != parent {
                commands.push(CanonicalCommand::SetLocalGroupParent {
                    key: before.key(),
                    parent,
                });
            }
        } else {
            commands.push(CanonicalCommand::CreateLocalGroup {
                key: LocalGroupKey {
                    definition_id: owner,
                    local_id: ids[&group.name],
                },
                name: group.name.clone(),
                transform: Transform::identity(),
                parent,
            });
        }
    }
    // apply_batch validates the final graph, so children may be reparented later in this batch.
    for (name, group) in &existing {
        if !ids.contains_key(name) {
            commands.push(CanonicalCommand::DeleteLocalGroup { key: group.key() });
        }
    }
    Ok(ids)
}

fn reconcile_members(
    document: &DocumentStore,
    old: &ProgramModel,
    new: &ProgramModel,
    rebuilt: &BTreeSet<String>,
) -> Result<Vec<CanonicalCommand>, RuleProgramApplyError> {
    let unsupported = || RuleProgramApplyError::IncrementalUnsupported;
    let snapshot = document.current();
    let added = new
        .components
        .iter()
        .flat_map(|component| &component.parts)
        .filter(|part| old.part(&part.name).is_none() || rebuilt.contains(&part.name))
        .cloned()
        .collect::<Vec<_>>();
    let planned =
        crate::planner::plan_rule_part_batch(document, &added).map_err(SessionError::Planning)?;
    let placements = planned
        .commands()
        .iter()
        .filter_map(|command| match command {
            CanonicalCommand::CreateOccurrence {
                name,
                definition_id,
                transform,
                tag,
                visible,
                ..
            } => Some((name.as_str(), (*definition_id, *transform, *tag, *visible))),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut commands = planned
        .commands()
        .iter()
        .filter(|command| !matches!(command, CanonicalCommand::CreateOccurrence { .. }))
        .cloned()
        .collect::<Vec<_>>();
    for before in &old.components {
        let after = new
            .components
            .iter()
            .find(|item| item.name == before.name)
            .ok_or_else(unsupported)?;
        let root = snapshot
            .definitions()
            .find(|item| item.name() == before.name)
            .ok_or_else(unsupported)?;
        let parents = reconcile_local_groups(&snapshot, root.id(), after, &mut commands)?;
        let mut next_id = snapshot
            .local_occurrences()
            .filter(|member| member.key().definition_id == root.id())
            .map(|member| member.key().local_id.0)
            .chain(
                snapshot
                    .local_groups()
                    .filter(|group| group.key().definition_id == root.id())
                    .map(|group| group.key().local_id.0),
            )
            .chain(parents.values().map(|id| id.0))
            .max()
            .unwrap_or(0);
        for removed in before
            .parts
            .iter()
            .filter(|part| !after.parts.iter().any(|item| item.name == part.name))
        {
            let member = snapshot
                .local_occurrences()
                .find(|member| {
                    member.key().definition_id == root.id() && member.name() == removed.name
                })
                .ok_or_else(unsupported)?;
            commands.push(CanonicalCommand::DeleteLocalOccurrence { key: member.key() });
            commands.push(CanonicalCommand::DeleteDefinition {
                id: member.definition_id(),
            });
        }
        for part in &after.parts {
            let member = snapshot
                .local_occurrences()
                .find(|item| item.key().definition_id == root.id() && item.name() == part.name);
            let parent = after
                .groups
                .iter()
                .find(|group| group.members.contains(&part.name))
                .ok_or_else(unsupported)?;
            let parent = if parent.name == after.name {
                None
            } else {
                Some(*parents.get(&parent.name).ok_or_else(unsupported)?)
            };
            if let Some(member) = member {
                if rebuilt.contains(&part.name) {
                    let (definition_id, _, _, _) =
                        *placements.get(part.name.as_str()).ok_or_else(unsupported)?;
                    commands.push(CanonicalCommand::RepointLocalOccurrence {
                        key: member.key(),
                        definition_id,
                    });
                    commands.push(CanonicalCommand::DeleteDefinition {
                        id: member.definition_id(),
                    });
                }
                if member.parent() != parent {
                    commands.push(CanonicalCommand::SetLocalOccurrenceParent {
                        key: member.key(),
                        parent,
                    });
                }
            } else {
                let (definition_id, transform, tag, visible) =
                    *placements.get(part.name.as_str()).ok_or_else(unsupported)?;
                next_id = next_id.checked_add(1).ok_or_else(unsupported)?;
                commands.push(CanonicalCommand::CreateLocalOccurrence {
                    key: ketchup_model::document::LocalOccurrenceKey {
                        definition_id: root.id(),
                        local_id: ketchup_model::document::LocalOccurrenceId(next_id),
                    },
                    definition_id,
                    name: part.name.clone(),
                    transform,
                    parent,
                    tag,
                    visible,
                });
            }
        }
        crate::rule_assembly_members::reconcile(
            &snapshot,
            old,
            new,
            &before.name,
            &parents,
            &mut commands,
        )?;
    }
    Ok(commands)
}

/// Updates shared geometry once and reconciles root instances without duplicating definitions.
/// Rebuilt member geometry is repointed beneath the existing local occurrence identity.
pub(crate) fn incremental(
    document: &DocumentStore,
    old: &ProgramModel,
    new: &ProgramModel,
) -> Result<CommandBatch, RuleProgramApplyError> {
    let unsupported = || RuleProgramApplyError::IncrementalUnsupported;
    let copied = crate::rule_instances::copied_parts(old);
    let new_copied = crate::rule_instances::copied_parts(new);
    let old_expanded = old;
    let new_expanded = new;
    let old = &crate::rule_assembly_members::direct_model(old);
    let new = &crate::rule_assembly_members::direct_model(new);
    for name in old
        .components
        .iter()
        .map(|c| &c.name)
        .chain(old.instances.iter().map(|i| &i.name))
    {
        if (new.components.iter().any(|c| &c.name == name)
            || new.instances.iter().any(|i| &i.name == name))
            && crate::rule_assembly_members::owner(old, name)
                != crate::rule_assembly_members::owner(new, name)
        {
            return Err(unsupported());
        }
    }
    let owners = |model: &ProgramModel| {
        model
            .components
            .iter()
            .flat_map(|component| {
                component
                    .parts
                    .iter()
                    .map(|part| (part.name.clone(), component.name.clone()))
            })
            .collect::<BTreeMap<_, _>>()
    };
    let before_owners = owners(old);
    let after_owners = owners(new);
    let before_roots = old
        .parts
        .iter()
        .filter(|part| !copied.contains(&part.name) && !before_owners.contains_key(&part.name))
        .map(|part| &part.name)
        .collect::<BTreeSet<_>>();
    let after_roots = new
        .parts
        .iter()
        .filter(|part| !new_copied.contains(&part.name) && !after_owners.contains_key(&part.name))
        .map(|part| &part.name)
        .collect::<BTreeSet<_>>();
    if old
        .components
        .iter()
        .map(|item| &item.name)
        .collect::<BTreeSet<_>>()
        != new
            .components
            .iter()
            .map(|item| &item.name)
            .collect::<BTreeSet<_>>()
        || before_owners
            .keys()
            .chain(before_roots.iter().copied())
            .any(|part| {
                new.part(part).is_some() && before_owners.get(part) != after_owners.get(part)
            })
    {
        return Err(unsupported());
    }
    let snapshot = document.current();
    let mut commands = Vec::new();
    let mut rebuilt = BTreeSet::new();
    for before in old
        .parts
        .iter()
        .filter(|part| before_owners.contains_key(&part.name) && new.part(&part.name).is_some())
    {
        let after = new
            .part(&before.name)
            .filter(|part| !new_copied.contains(&part.name))
            .ok_or_else(unsupported)?;
        let same_boolean_layout = before.booleans().eq(after.booleans())
            && (before.booleans().next().is_none()
                || before.transform_matrix() == after.transform_matrix());
        let mut comparable = after.clone();
        comparable.at_mm = before.at_mm;
        comparable.grounded = before.grounded;
        comparable.color = before.color;
        comparable.rotation = before.rotation;
        comparable.size_mm = before.size_mm;
        comparable.body = before.body.clone();
        comparable.operations = before.operations.clone();
        comparable.features = before.features.clone();
        if comparable != *before {
            return Err(unsupported());
        }
        let definition = {
            let component = old
                .components
                .iter()
                .find(|component| component.parts.iter().any(|part| part.name == before.name))
                .ok_or_else(unsupported)?;
            let root = snapshot
                .definitions()
                .find(|item| item.name() == component.name)
                .ok_or_else(unsupported)?;
            let member = snapshot
                .local_occurrences()
                .find(|item| item.key().definition_id == root.id() && item.name() == before.name)
                .ok_or_else(unsupported)?;
            if member.transform() != transform(before.at_mm, before.rotation)? {
                return Err(unsupported());
            }
            if before.transform_matrix() != after.transform_matrix() {
                commands.push(CanonicalCommand::SetLocalOccurrenceTransform {
                    key: member.key(),
                    transform: transform(after.at_mm, after.rotation)?,
                });
            }
            member.definition_id()
        };
        let feature_commands = (same_boolean_layout
            && crate::rule_program::program_feature_references_match(before, after))
        .then(|| {
            crate::rule_program::program_feature_commands(&snapshot, definition, before, after)
        })
        .flatten();
        if let Some(changes) = feature_commands {
            commands.extend(changes);
        } else {
            rebuilt.insert(before.name.clone());
        }
    }
    let mut structural = reconcile_members(document, old, new, &rebuilt)?;
    structural.extend(commands);
    let mut staged = document.fork_for_planning();
    if !structural.is_empty() {
        staged
            .apply_batch(&CommandBatch::new(structural.clone()))
            .map_err(SessionError::Canonical)?;
    }
    let root_model = |model: &ProgramModel, names: &BTreeSet<&String>| {
        let mut roots = model.clone();
        roots.parts.retain(|p| names.contains(&p.name));
        roots
    };
    let root_commands = crate::rule_program::incremental_parts(
        &staged,
        &root_model(old, &before_roots),
        &root_model(new, &after_roots),
    )?
    .ok_or_else(unsupported)?;
    structural.extend(root_commands.commands().iter().cloned());
    crate::rule_instances::reconcile(&snapshot, old_expanded, new_expanded, structural)
}
