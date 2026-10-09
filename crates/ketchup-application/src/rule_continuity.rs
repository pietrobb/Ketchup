//! Plan identity renames before geometry changes, publishing both in the same batch.
use crate::{RuleProgramApplyError, SessionError, rule_assembly_members};
use ketchup_model::document::{CanonicalCommand, CommandBatch, DocumentStore, Snapshot};
use ketchup_program::ProgramModel;
use std::collections::BTreeMap;

pub(crate) struct Prepared {
    pub document: DocumentStore,
    pub model: ProgramModel,
    pub commands: Vec<CanonicalCommand>,
}

fn invalid(name: &str, previous: &str, reason: &str) -> RuleProgramApplyError {
    RuleProgramApplyError::Program {
        code: "invalid_part_continuity".into(),
        message: format!(
            "continue_part({name:?}, was={previous:?}): {reason}; use the name of one existing previous part, or remove was for a new part"
        ),
        location: None,
    }
}

fn rename_commands(
    snapshot: &Snapshot,
    owner: Option<&str>,
    previous: &str,
    name: &str,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), RuleProgramApplyError> {
    let absent = || {
        invalid(
            name,
            previous,
            "the previous part is absent from the document",
        )
    };
    let definition = if let Some(owner) = owner {
        let component = snapshot
            .definitions()
            .find(|d| d.name() == owner)
            .ok_or_else(absent)?;
        let member = snapshot
            .local_occurrences()
            .find(|p| p.key().definition_id == component.id() && p.name() == previous)
            .ok_or_else(absent)?;
        commands.push(CanonicalCommand::RenameLocalOccurrence {
            key: member.key(),
            name: name.to_owned(),
        });
        member.definition_id()
    } else {
        let occurrence = snapshot
            .occurrences()
            .find(|p| p.name() == previous)
            .ok_or_else(absent)?;
        commands.push(CanonicalCommand::RenameEntity {
            id: occurrence.id(),
            name: name.to_owned(),
        });
        occurrence.definition_id()
    };
    commands.push(CanonicalCommand::RenameDefinition {
        id: definition,
        name: name.to_owned(),
    });
    Ok(())
}

fn align_names(model: &mut ProgramModel, mut names: BTreeMap<String, String>) {
    // Instances are in declaration order, so an enclosing instance sees all child renames.
    for instance in &model.instances {
        if let Some(component) = model
            .components
            .iter()
            .find(|c| c.name == instance.component)
        {
            let expanded = component
                .parts
                .iter()
                .filter_map(|p| {
                    names.get(&p.name).map(|name| {
                        (
                            format!("{}/{}", instance.name, p.name),
                            format!("{}/{}", instance.name, name),
                        )
                    })
                })
                .collect::<Vec<_>>();
            names.extend(expanded);
        }
    }
    let rename = |name: &mut String| {
        if let Some(next) = names.get(name) {
            name.clone_from(next);
        }
    };
    for part in &mut model.parts {
        rename(&mut part.name);
    }
    for group in &mut model.groups {
        for member in &mut group.members {
            rename(member);
        }
    }
    for joint in &mut model.joints {
        for part in &mut joint.parts {
            rename(part);
        }
    }
    for component in &mut model.components {
        for part in &mut component.parts {
            rename(&mut part.name);
        }
        for group in &mut component.groups {
            for member in &mut group.members {
                rename(member);
            }
        }
        for joint in &mut component.joints {
            for part in &mut joint.parts {
                rename(part);
            }
        }
    }
}

pub(crate) fn prepare(
    document: &DocumentStore,
    mut old: ProgramModel,
    new: &ProgramModel,
) -> Result<Prepared, RuleProgramApplyError> {
    if new.continuations.is_empty() {
        return Ok(Prepared {
            document: document.fork_for_planning(),
            model: old,
            commands: Vec::new(),
        });
    }
    let snapshot = document.current();
    let mut commands = Vec::new();
    let mut names = BTreeMap::new();
    let old_direct = rule_assembly_members::direct_model(&old);
    let new_direct = rule_assembly_members::direct_model(new);
    let old_copies = crate::rule_instances::copied_parts(&old);
    let new_copies = crate::rule_instances::copied_parts(new);
    for (name, previous) in &new.continuations {
        if new_copies.contains(name) || old_copies.contains(previous) {
            return Err(invalid(
                name,
                previous,
                "an instance leaf cannot claim identity independently; declare continue_part on the shared source member",
            ));
        }
        if old.part(name).is_some() {
            if name != previous && old.part(previous).is_some() {
                return Err(invalid(
                    name,
                    previous,
                    "both names already exist in the previous program",
                ));
            }
            if name != previous && old.continuations.get(name) != Some(previous) {
                return Err(invalid(
                    name,
                    previous,
                    "this existing part has no matching previous identity declaration",
                ));
            }
            continue;
        }
        if old.part(previous).is_none() {
            return Err(invalid(
                name,
                previous,
                "the previous program has no such part",
            ));
        }
        let owner = rule_assembly_members::owner(&old_direct, previous);
        if owner != rule_assembly_members::owner(&new_direct, name) {
            return Err(invalid(
                name,
                previous,
                "the shared definition owner changed; keep the continuation in the same component",
            ));
        }
        rename_commands(&snapshot, owner, previous, name, &mut commands)?;
        names.insert(previous.clone(), name.clone());
    }
    align_names(&mut old, names);
    let mut staged = document.fork_for_planning();
    if !commands.is_empty() {
        staged
            .apply_batch(&CommandBatch::new(commands.clone()))
            .map_err(SessionError::Canonical)?;
    }
    Ok(Prepared {
        document: staged,
        model: old,
        commands,
    })
}
