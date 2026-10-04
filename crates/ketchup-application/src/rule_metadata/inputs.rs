//! Project generic numeric program inputs into the existing evaluator graph.
use super::*;
use ketchup_model::document::Dimension;
use ketchup_model::document::NodeId;

fn rejected(target: &str, reason: &str, cause: Option<String>) -> RuleProgramApplyError {
    let mut rejection = assistant_planning_rejection(
        "invalid_program_input",
        "program.apply",
        target,
        reason,
        "Use finite input:<name> values; use {occurrence} for the owning root ID, and give each input one consistent value.",
    );
    rejection.causes.extend(cause);
    SessionError::Planning(rejection).into()
}

fn values(
    model: &ProgramModel,
    names: &BTreeMap<String, InstancePath>,
) -> Result<BTreeMap<String, f64>, RuleProgramApplyError> {
    let mut result = BTreeMap::new();
    for part in &model.parts {
        let Some(path) = names.get(&part.name).filter(|path| path.steps().is_empty()) else {
            continue;
        };
        for (key, value) in &part.attributes {
            let Some(name) = key.strip_prefix("input:") else {
                continue;
            };
            let target = format!("{}.attributes.{key}", part.name);
            let name = name.replace("{occurrence}", &path.root_occurrence().0.to_string());
            let value = value.parse::<f64>().map_err(|error| {
                rejected(
                    &target,
                    "The input value must be a finite number.",
                    Some(error.to_string()),
                )
            })?;
            if name.trim().is_empty() || !value.is_finite() {
                return Err(rejected(
                    &target,
                    "The input needs a nonempty name and a finite value.",
                    None,
                ));
            }
            if let Some(previous) = result.insert(name, value)
                && previous != value
            {
                return Err(rejected(
                    &target,
                    "Multiple parts declare conflicting values for the same input.",
                    None,
                ));
            }
        }
    }
    Ok(result)
}

pub(super) fn append(
    old_names: &BTreeMap<String, InstancePath>,
    snapshot: &Snapshot,
    old: Option<&ProgramModel>,
    model: &ProgramModel,
    names: &BTreeMap<String, InstancePath>,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), RuleProgramApplyError> {
    let previous = old
        .map(|model| values(model, old_names))
        .transpose()?
        .unwrap_or_default();
    let desired = values(model, names)?;
    let mut next = snapshot
        .evaluator_node_ids()
        .map(|id| id.0)
        .max()
        .unwrap_or(0);
    for name in previous
        .keys()
        .chain(desired.keys())
        .collect::<BTreeSet<_>>()
    {
        let mut matches = snapshot
            .evaluator_nodes()
            .filter(|node| node.name() == name);
        let existing = matches.next();
        if matches.next().is_some() || (existing.is_some() && !previous.contains_key(name)) {
            return Err(rejected(
                name,
                "The input name is ambiguous or belongs to an existing non-program input.",
                None,
            ));
        }
        match (existing, desired.get(name)) {
            (Some(node), Some(value)) => {
                if node.dimension().is_none() || !node.dependencies().is_empty() {
                    return Err(rejected(
                        name,
                        "Only independent numeric inputs may be updated by program attributes.",
                        None,
                    ));
                }
                if node
                    .dimension()
                    .is_some_and(|dimension| dimension.millimetres() == *value)
                {
                    continue;
                }
                commands.push(CanonicalCommand::SetEvaluatorDimension {
                    id: node.id(),
                    dimension: Dimension::new(value.to_string(), *value)
                        .map_err(|error| SessionError::Canonical(error.into()))?,
                });
            }
            (None, Some(value)) => {
                commands.push(CanonicalCommand::CreateEvaluatorNode {
                    id: NodeId(next_id(&mut next)?),
                    name: name.clone(),
                    dimension: Dimension::new(value.to_string(), *value)
                        .map_err(|error| SessionError::Canonical(error.into()))?,
                    dependencies: Vec::new(),
                });
            }
            (Some(node), None) => {
                commands.push(CanonicalCommand::DeleteEvaluatorNode { id: node.id() })
            }
            (None, None) => {}
        }
    }
    Ok(())
}
