//! Reconcile the tags (layers) a program names on its root parts, by tag name.
//!
//! The program owns only the tags it names: re-applying removes a tag the previous
//! source named and the current one no longer does, keeps tags the user added by
//! hand, creates missing tags visible, and deletes a tag the program stopped naming
//! once no part uses it.
use crate::{RuleProgramApplyError, SessionError, diagnostics::assistant_planning_rejection};
use ketchup_model::document::{CanonicalCommand, InstancePath, OccurrenceId, Snapshot, TagId};
use ketchup_program::ProgramModel;
use std::collections::{BTreeMap, BTreeSet};

fn names(model: &ProgramModel) -> BTreeSet<&str> {
    model
        .parts
        .iter()
        .flat_map(|part| &part.tags)
        .map(String::as_str)
        .collect()
}

pub(crate) fn needed(model: &ProgramModel) -> bool {
    model.parts.iter().any(|part| !part.tags.is_empty())
}

fn tag_named(snapshot: &Snapshot, name: &str) -> Result<Option<TagId>, RuleProgramApplyError> {
    let mut matches = snapshot.tags().filter(|tag| tag.name() == name);
    let first = matches.next().map(|tag| tag.id());
    if matches.next().is_some() {
        return Err(SessionError::Planning(assistant_planning_rejection(
            "ambiguous_program_tag",
            "program.apply",
            name,
            "A program tag must identify exactly one document tag.",
            "Give tags unique names before applying the program.",
        ))
        .into());
    }
    Ok(first)
}

pub(crate) fn append(
    snapshot: &Snapshot,
    old: Option<&ProgramModel>,
    model: &ProgramModel,
    paths: &BTreeMap<String, InstancePath>,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), RuleProgramApplyError> {
    let old_names = old.map(names).unwrap_or_default();
    let new_names = names(model);
    let mut ids = BTreeMap::new();
    let mut next = snapshot.tags().map(|tag| tag.id().0).max().unwrap_or(0);
    for name in old_names.union(&new_names) {
        let id = match tag_named(snapshot, name)? {
            Some(id) => Some(id),
            None if new_names.contains(name) => {
                next = next.checked_add(1).ok_or(SessionError::Canonical(
                    ketchup_model::document::CanonicalError::IdExhausted,
                ))?;
                commands.push(CanonicalCommand::CreateTag {
                    id: TagId(next),
                    name: (*name).to_owned(),
                    visible: true,
                });
                Some(TagId(next))
            }
            None => None,
        };
        ids.insert(*name, id);
    }
    let resolve = |tags: &BTreeSet<String>| {
        tags.iter()
            .filter_map(|name| ids.get(name.as_str()).copied().flatten())
            .collect::<BTreeSet<_>>()
    };
    let previous = old
        .into_iter()
        .flat_map(|old| &old.parts)
        .map(|part| (part.name.as_str(), resolve(&part.tags)))
        .collect::<BTreeMap<_, _>>();
    let mut changed = BTreeMap::<OccurrenceId, BTreeSet<TagId>>::new();
    for part in &model.parts {
        // Shared component leaves are not root occurrences; their tags come from
        // the component definition, like classifications.
        let Some(path) = paths.get(&part.name).filter(|path| path.steps().is_empty()) else {
            continue;
        };
        let Some(occurrence) = snapshot.occurrence(path.root_occurrence()) else {
            continue;
        };
        let owned_before = previous.get(part.name.as_str());
        let mut tags = occurrence
            .tags()
            .iter()
            .filter(|id| owned_before.is_none_or(|owned| !owned.contains(id)))
            .copied()
            .collect::<BTreeSet<_>>();
        tags.extend(resolve(&part.tags));
        if &tags != occurrence.tags() {
            changed.insert(occurrence.id(), tags);
        }
    }
    for (id, tags) in &changed {
        commands.push(CanonicalCommand::SetOccurrenceTags {
            id: *id,
            tags: tags.clone(),
        });
    }
    for name in old_names.difference(&new_names) {
        let Some(Some(id)) = ids.get(name) else {
            continue;
        };
        let in_use = snapshot.occurrences().any(|occurrence| {
            changed
                .get(&occurrence.id())
                .unwrap_or(occurrence.tags())
                .contains(id)
        }) || snapshot
            .local_occurrences()
            .any(|occurrence| occurrence.tags().contains(id));
        if !in_use {
            commands.push(CanonicalCommand::DeleteTag { id: *id });
        }
    }
    Ok(())
}
