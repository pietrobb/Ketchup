//! Reconcile program classifications on canonical root occurrences, after geometry identities settle.
mod inputs;
use crate::{RuleProgramApplyError, SessionError, diagnostics::assistant_planning_rejection};
use ketchup_model::document::{
    CanonicalCommand, CanonicalError, ClassificationCategoryId, ClassificationDimensionId,
    InstancePath, OccurrenceId, Snapshot,
};
use ketchup_model::validation::MATERIAL_DIMENSION_V1;
use ketchup_program::{ProgramModel, model::Part};
use std::collections::{BTreeMap, BTreeSet};

fn classifications(part: &Part) -> BTreeMap<&str, &str> {
    let mut values = part
        .attributes
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("classification:")
                .map(|name| (name, value.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    if let Some(material) = part.material.as_deref() {
        values.insert(MATERIAL_DIMENSION_V1, material);
    }
    values
}

pub(crate) fn needed(model: &ProgramModel) -> bool {
    model.parts.iter().any(|part| {
        !classifications(part).is_empty()
            || part.attributes.keys().any(|key| key.starts_with("input:"))
    })
}

pub(crate) fn append(
    before: &Snapshot,
    snapshot: &Snapshot,
    old: Option<&ProgramModel>,
    model: &ProgramModel,
    names: &BTreeMap<String, InstancePath>,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), RuleProgramApplyError> {
    let old_names = before
        .scene_query()
        .into_iter()
        .filter_map(|leaf| {
            crate::rule_program_part_name(before, &leaf.instance_path)
                .map(|name| (name, leaf.instance_path))
        })
        .collect::<BTreeMap<_, _>>();
    let mut desired = BTreeMap::<String, BTreeMap<OccurrenceId, Option<String>>>::new();
    // Clear only assignments previously owned by the source, never other dimensions.
    for (previous, model) in old
        .map(|old| (true, old))
        .into_iter()
        .chain([(false, model)])
    {
        for part in &model.parts {
            let Some(path) = (if previous { &old_names } else { names }).get(&part.name) else {
                continue;
            };
            // Canonical classifications do not yet address shared leaves. Never classify
            // their assembly root as if it were one of its members.
            if !path.steps().is_empty() || snapshot.occurrence(path.root_occurrence()).is_none() {
                continue;
            }
            for (name, value) in classifications(part) {
                desired.entry(name.to_owned()).or_default().insert(
                    path.root_occurrence(),
                    (!previous).then(|| value.to_owned()),
                );
            }
        }
    }
    let mut next_dimension = snapshot
        .classification_dimensions()
        .map(|d| d.id().0)
        .max()
        .unwrap_or(0);
    for (name, assignments) in desired {
        reconcile_dimension(snapshot, &name, assignments, &mut next_dimension, commands)?;
    }
    inputs::append(&old_names, snapshot, old, model, names, commands)
}

fn next_id(value: &mut u64) -> Result<u64, RuleProgramApplyError> {
    *value = value
        .checked_add(1)
        .ok_or(SessionError::Canonical(CanonicalError::IdExhausted))?;
    Ok(*value)
}

fn reconcile_dimension(
    snapshot: &Snapshot,
    name: &str,
    assignments: BTreeMap<OccurrenceId, Option<String>>,
    next_dimension: &mut u64,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), RuleProgramApplyError> {
    let existing = snapshot
        .classification_dimension_named(name)
        .map_err(|error| {
            let mut rejection = assistant_planning_rejection(
                "ambiguous_program_classification",
                "program.apply",
                name,
                "A program classification must identify exactly one canonical dimension.",
                "Give classification dimensions unique names before applying the program.",
            );
            rejection.causes.push(error.to_string());
            SessionError::Planning(rejection)
        })?;
    let values = assignments
        .values()
        .filter_map(Option::as_ref)
        .collect::<BTreeSet<_>>();
    if existing.is_none() && values.is_empty() {
        return Ok(());
    }
    let id = match existing {
        Some(dimension) => dimension.id(),
        None => ClassificationDimensionId(next_id(next_dimension)?),
    };
    let mut categories = existing
        .into_iter()
        .flat_map(|dimension| dimension.categories())
        .map(|category| (category.id(), category.name().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let mut by_name = categories
        .iter()
        .map(|(id, name)| (name.clone(), *id))
        .collect::<BTreeMap<_, _>>();
    let mut next_category = categories.keys().map(|id| id.0).max().unwrap_or(0);
    let mut changed = existing.is_none();
    for value in values {
        if !by_name.contains_key(value) {
            let category = ClassificationCategoryId(next_id(&mut next_category)?);
            categories.insert(category, value.clone());
            by_name.insert(value.clone(), category);
            changed = true;
        }
    }
    if changed {
        commands.push(CanonicalCommand::UpsertClassificationDimension {
            id,
            name: name.to_owned(),
            categories: categories.into_iter().collect(),
        });
    }
    for (occurrence_id, value) in assignments {
        let category_id = value.as_ref().map(|value| by_name[value]);
        if snapshot.occurrence_classification(occurrence_id, id) != category_id {
            commands.push(CanonicalCommand::SetOccurrenceClassification {
                occurrence_id,
                dimension_id: id,
                category_id,
            });
        }
    }
    Ok(())
}
