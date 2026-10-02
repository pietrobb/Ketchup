use super::*;

pub(super) fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, CanonicalError> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(CanonicalError::IdExhausted)
}

pub(super) fn group_is_descendant(product: &ProductModel, root: GroupId, target: GroupId) -> bool {
    let mut cursor = Some(target);
    let mut visited = BTreeSet::new();
    while let Some(candidate) = cursor {
        if !visited.insert(candidate) {
            return false;
        }
        if candidate == root {
            return true;
        }
        cursor = product
            .groups
            .get(&candidate)
            .and_then(|group| group.parent);
    }
    false
}

pub(super) fn descendant_groups(
    product: &ProductModel,
    root: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    if !product.groups.contains_key(&root) {
        return Err(CanonicalError::GroupNotFound(root));
    }
    Ok(product
        .groups
        .keys()
        .cloned()
        .filter(|id| group_is_descendant(product, root, *id))
        .collect())
}

pub(super) fn world_group_lineage(
    product: &ProductModel,
    target: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    let mut lineage = Vec::new();
    let mut cursor = Some(target);
    let mut visited = BTreeSet::new();
    while let Some(id) = cursor {
        if !visited.insert(id) {
            return Err(CanonicalError::GroupCycle(id));
        }
        let group = product
            .groups
            .get(&id)
            .ok_or(CanonicalError::GroupNotFound(id))?;
        lineage.push(id);
        cursor = group.parent;
    }
    lineage.reverse();
    Ok(lineage)
}

pub(super) fn group_lineage(
    product: &ProductModel,
    root: GroupId,
    target: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    let mut lineage = Vec::new();
    let mut cursor = Some(target);
    let mut visited = BTreeSet::new();
    while let Some(id) = cursor {
        if !visited.insert(id) {
            return Err(CanonicalError::GroupCycle(id));
        }
        lineage.push(id);
        if id == root {
            lineage.reverse();
            return Ok(lineage);
        }
        cursor = product.groups.get(&id).and_then(|group| group.parent);
    }
    Err(CanonicalError::InvalidLocalGraph)
}

pub(super) fn conversion_mappings(
    product: &ProductModel,
    plan: &ConvertGroupPlan,
) -> Result<Vec<ConversionMapping>, CanonicalError> {
    let converted_groups = descendant_groups(product, plan.group_id)?;
    let converted_group_set: BTreeSet<_> = converted_groups.iter().cloned().collect();
    let mut mappings = Vec::new();
    for id in product.groups.keys().cloned() {
        let old_path = WorldEntityPath {
            groups: world_group_lineage(product, id)?,
            occurrence: None,
        };
        let resolution = if converted_group_set.contains(&id) {
            let converted_lineage = group_lineage(product, plan.group_id, id)?;
            let mut new_path = InstancePath::root(plan.new_occurrence_id);
            new_path.steps.extend(
                converted_lineage
                    .iter()
                    .skip(1)
                    .map(|id| InstancePathStep::Group(LocalGroupId(id.0))),
            );
            MappingResolution::Resolved {
                new_id: if id == plan.group_id {
                    ConvertedEntityId::ComponentOccurrence(plan.new_occurrence_id)
                } else {
                    ConvertedEntityId::LocalGroup(LocalGroupKey {
                        definition_id: plan.new_definition_id,
                        local_id: LocalGroupId(id.0),
                    })
                },
                new_path,
            }
        } else {
            MappingResolution::Unresolved {
                reason: UnresolvedMappingReason::NotInConvertedGroup,
            }
        };
        mappings.push(ConversionMapping {
            old_id: WorldEntityId::Group(id),
            old_path,
            resolution,
        });
    }
    for occurrence in product.occurrences.values() {
        let old_groups = occurrence.parent.map_or_else(
            || Ok(Vec::new()),
            |parent| world_group_lineage(product, parent),
        )?;
        let old_path = WorldEntityPath {
            groups: old_groups,
            occurrence: Some(occurrence.id),
        };
        let resolution = if let Some(parent) = occurrence
            .parent
            .filter(|parent| converted_group_set.contains(parent))
        {
            let converted_lineage = group_lineage(product, plan.group_id, parent)?;
            let mut new_path = InstancePath::root(plan.new_occurrence_id);
            new_path.steps.extend(
                converted_lineage
                    .iter()
                    .skip(1)
                    .map(|id| InstancePathStep::Group(LocalGroupId(id.0))),
            );
            new_path
                .steps
                .push(InstancePathStep::Occurrence(LocalOccurrenceId(
                    occurrence.id.0,
                )));
            MappingResolution::Resolved {
                new_id: ConvertedEntityId::LocalOccurrence(LocalOccurrenceKey {
                    definition_id: plan.new_definition_id,
                    local_id: LocalOccurrenceId(occurrence.id.0),
                }),
                new_path,
            }
        } else {
            MappingResolution::Unresolved {
                reason: UnresolvedMappingReason::NotInConvertedGroup,
            }
        };
        mappings.push(ConversionMapping {
            old_id: WorldEntityId::Occurrence(occurrence.id),
            old_path,
            resolution,
        });
    }
    mappings.sort_by(|left, right| left.old_path.cmp(&right.old_path));
    Ok(mappings)
}

pub(super) fn convert_group_to_component_model(
    product: &mut ProductModel,
    plan: &ConvertGroupPlan,
) -> Result<(), CanonicalError> {
    ensure_name(&plan.component_name)?;
    if product.definitions.contains_key(&plan.new_definition_id) {
        return Err(CanonicalError::DefinitionAlreadyExists(
            plan.new_definition_id,
        ));
    }
    if product.occurrences.contains_key(&plan.new_occurrence_id) {
        return Err(CanonicalError::OccurrenceAlreadyExists(
            plan.new_occurrence_id,
        ));
    }
    let root = product
        .groups
        .get(&plan.group_id)
        .ok_or(CanonicalError::GroupNotFound(plan.group_id))?
        .as_ref()
        .clone();
    let groups = descendant_groups(product, plan.group_id)?;
    let group_set: BTreeSet<_> = groups.iter().cloned().collect();
    let occurrence_ids: Vec<_> = product
        .occurrences
        .values()
        .filter(|item| {
            item.parent
                .is_some_and(|parent| group_set.contains(&parent))
        })
        .map(|item| item.id)
        .collect();
    let local_group_ids = groups
        .iter()
        .cloned()
        .filter(|id| *id != plan.group_id)
        .map(|id| LocalGroupId(id.0))
        .collect::<Vec<_>>();
    let local_occurrence_ids = occurrence_ids
        .iter()
        .map(|id| LocalOccurrenceId(id.0))
        .collect::<Vec<_>>();
    let inherited_classifications = product
        .classification_dimensions
        .keys()
        .filter_map(|dimension_id| {
            let category_id = product
                .classification_assignments
                .get(&(*occurrence_ids.first()?, *dimension_id))
                .copied()?;
            occurrence_ids
                .iter()
                .all(|occurrence_id| {
                    product
                        .classification_assignments
                        .get(&(*occurrence_id, *dimension_id))
                        == Some(&category_id)
                })
                .then_some((*dimension_id, category_id))
        })
        .collect::<Vec<_>>();
    product.definitions.insert(
        plan.new_definition_id,
        Arc::new(Definition {
            local_occurrence_ids: local_occurrence_ids.clone(),
            local_group_ids: local_group_ids.clone(),
            ..new_definition(plan.new_definition_id, plan.component_name.clone())
        }),
    );
    for id in groups.iter().cloned().filter(|id| *id != plan.group_id) {
        let group = product.groups[&id].as_ref();
        let key = LocalGroupKey {
            definition_id: plan.new_definition_id,
            local_id: LocalGroupId(id.0),
        };
        product.local_groups.insert(
            key,
            Arc::new(LocalGroup {
                key,
                name: group.name.clone(),
                transform: group.transform,
                parent: group
                    .parent
                    .filter(|parent| *parent != plan.group_id)
                    .map(|parent| LocalGroupId(parent.0)),
            }),
        );
    }
    for id in &occurrence_ids {
        let occurrence = product.occurrences[id].as_ref();
        let key = LocalOccurrenceKey {
            definition_id: plan.new_definition_id,
            local_id: LocalOccurrenceId(id.0),
        };
        product.local_occurrences.insert(
            key,
            Arc::new(LocalOccurrence {
                key,
                definition_id: occurrence.definition_id,
                name: occurrence.name.clone(),
                transform: occurrence.transform,
                parent: occurrence
                    .parent
                    .filter(|parent| *parent != plan.group_id)
                    .map(|parent| LocalGroupId(parent.0)),
                tag: occurrence.tag,
                visible: occurrence.visible,
                color: occurrence.color,
            }),
        );
    }
    let converted_occurrence_ids = occurrence_ids.iter().copied().collect::<BTreeSet<_>>();
    for id in occurrence_ids {
        product.occurrences.remove(&id);
    }
    product
        .classification_assignments
        .retain(|(occurrence_id, _), _| !converted_occurrence_ids.contains(occurrence_id));
    for (dimension_id, category_id) in inherited_classifications {
        product
            .classification_assignments
            .insert((plan.new_occurrence_id, dimension_id), category_id);
    }
    for id in groups {
        product.groups.remove(&id);
    }
    product.occurrences.insert(
        plan.new_occurrence_id,
        Arc::new(Occurrence {
            id: plan.new_occurrence_id,
            definition_id: plan.new_definition_id,
            name: plan.component_name.clone(),
            transform: root.transform,
            parent: root.parent,
            tag: None,
            visible: true,
            color: None,
        }),
    );
    Ok(())
}

pub(super) fn refresh_override_health(product: &mut ProductModel) {
    for value in product.overrides.values_mut() {
        let audited = resolve_derived_identity(&product.evaluator_nodes, &value.target);
        if value.health != audited {
            let mut refreshed = value.as_ref().clone();
            refreshed.health = audited;
            *value = Arc::new(refreshed);
        }
    }
}

pub(super) fn validate_overrides(product: &ProductModel) -> Result<(), CanonicalError> {
    for value in product.overrides.values() {
        if let Some(root) = product.evaluator_nodes.get(&value.target.root_rule_node_id)
            && !root
                .allowed_parameters()
                .iter()
                .any(|parameter| parameter.name() == value.parameter)
        {
            return Err(CanonicalError::UndeclaredOverrideParameter);
        }
    }
    Ok(())
}
