//! Occurrences, groups, their local copies, and the solid tools that repoint them.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::CreateOccurrence {
            id,
            definition_id,
            name,
            transform,
            parent,
            tags,
            visible,
        } => {
            group_conversion::create_occurrence(
                &mut edit.product,
                Occurrence {
                    id: *id,
                    definition_id: *definition_id,
                    name: name.clone(),
                    transform: *transform,
                    parent: *parent,
                    tags: tags.clone(),
                    visible: *visible,
                    color: None,
                },
            )?;
        }
        CanonicalCommand::DeleteOccurrence { id } => {
            group_conversion::delete_occurrence(&mut edit.product, *id)?;
        }
        CanonicalCommand::SetOccurrenceTransform { id, transform } => {
            validate_transform(*transform)?;
            let existing = edit
                .product
                .occurrences
                .get_mut(id)
                .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
            Arc::make_mut(existing).transform = *transform;
        }
        CanonicalCommand::CreateLocalOccurrence {
            key,
            definition_id,
            name,
            transform,
            parent,
            tags,
            visible,
        } => {
            group_conversion::create_local_occurrence(
                &mut edit.product,
                LocalOccurrence {
                    key: *key,
                    definition_id: *definition_id,
                    name: name.clone(),
                    transform: *transform,
                    parent: *parent,
                    tags: tags.clone(),
                    visible: *visible,
                    color: None,
                },
            )?;
        }
        CanonicalCommand::DeleteLocalOccurrence { key } => {
            group_conversion::delete_local_occurrence(&mut edit.product, *key)?;
        }
        CanonicalCommand::RepointLocalOccurrence { key, definition_id } => {
            group_conversion::local_occurrence_mut(&mut edit.product, *key)?.definition_id =
                *definition_id;
        }
        CanonicalCommand::SetLocalOccurrenceParent { key, parent } => {
            group_conversion::local_occurrence_mut(&mut edit.product, *key)?.parent = *parent;
        }
        CanonicalCommand::SetLocalOccurrenceTransform { key, transform } => {
            validate_transform(*transform)?;
            group_conversion::local_occurrence_mut(&mut edit.product, *key)?.transform = *transform;
        }
        CanonicalCommand::RenameLocalOccurrence { key, name } => {
            group_conversion::rename_local_occurrence(&mut edit.product, *key, name)?;
        }
        CanonicalCommand::RenameEntity { id, name } => {
            group_conversion::rename_occurrence(&mut edit.product, *id, name)?;
        }
        CanonicalCommand::SetOccurrenceColor { id, color } => {
            let occurrence = edit
                .product
                .occurrences
                .get_mut(id)
                .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
            Arc::make_mut(occurrence).color = *color;
        }
        CanonicalCommand::SetLocalOccurrenceColor { key, color } => {
            group_conversion::local_occurrence_mut(&mut edit.product, *key)?.color = *color;
        }
        CanonicalCommand::SetLocalOccurrenceTags { key, tags } => {
            set_local_occurrence_tags(&mut edit.product, *key, tags)?;
        }
        CanonicalCommand::SetOccurrenceVisibility { id, visible } => {
            let existing = edit
                .product
                .occurrences
                .get(id)
                .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
            edit.product.occurrences.insert(
                *id,
                Arc::new(Occurrence {
                    visible: *visible,
                    ..existing.as_ref().clone()
                }),
            );
        }
        CanonicalCommand::SetOccurrenceTags { id, tags } => {
            set_occurrence_tags(&mut edit.product, *id, tags)?;
        }
        CanonicalCommand::RepointOccurrence { id, definition_id } => {
            let existing = edit
                .product
                .occurrences
                .get(id)
                .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
            edit.product.occurrences.insert(
                *id,
                Arc::new(Occurrence {
                    definition_id: *definition_id,
                    ..existing.as_ref().clone()
                }),
            );
        }
        CanonicalCommand::SetOccurrenceParent { id, parent } => {
            let existing = edit
                .product
                .occurrences
                .get(id)
                .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
            edit.product.occurrences.insert(
                *id,
                Arc::new(Occurrence {
                    parent: *parent,
                    ..existing.as_ref().clone()
                }),
            );
        }
        CanonicalCommand::CreateGroup {
            id,
            name,
            transform,
            parent,
        } => {
            ensure_product_id(id.0)?;
            ensure_name(name)?;
            validate_transform(*transform)?;
            if edit.product.groups.contains_key(id) {
                return Err(CanonicalError::GroupAlreadyExists(*id));
            }
            edit.product.groups.insert(
                *id,
                Arc::new(Group {
                    id: *id,
                    name: name.clone(),
                    transform: *transform,
                    parent: *parent,
                }),
            );
        }
        CanonicalCommand::DeleteGroup { id } => {
            group_conversion::delete_group(&mut edit.product, *id)?;
        }
        CanonicalCommand::CreateLocalGroup {
            key,
            name,
            transform,
            parent,
        } => {
            group_conversion::create_local_group(
                &mut edit.product,
                LocalGroup {
                    key: *key,
                    name: name.clone(),
                    transform: *transform,
                    parent: *parent,
                },
            )?;
        }
        CanonicalCommand::DeleteLocalGroup { key } => {
            group_conversion::delete_local_group(&mut edit.product, *key)?;
        }
        CanonicalCommand::SetLocalGroupTransform { key, transform } => {
            validate_transform(*transform)?;
            group_conversion::local_group_mut(&mut edit.product, *key)?.transform = *transform;
        }
        CanonicalCommand::SetLocalGroupParent { key, parent } => {
            group_conversion::local_group_mut(&mut edit.product, *key)?.parent = *parent;
        }
        CanonicalCommand::SetGroupTransform { id, transform } => {
            validate_transform(*transform)?;
            let existing = edit
                .product
                .groups
                .get_mut(id)
                .ok_or(CanonicalError::GroupNotFound(*id))?;
            Arc::make_mut(existing).transform = *transform;
        }
        CanonicalCommand::SetGroupParent { id, parent } => {
            let existing = edit
                .product
                .groups
                .get(id)
                .ok_or(CanonicalError::GroupNotFound(*id))?;
            edit.product.groups.insert(
                *id,
                Arc::new(Group {
                    parent: *parent,
                    ..existing.as_ref().clone()
                }),
            );
        }
        CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
            clone_definition_and_repoint(&mut edit.product, plan)?;
        }
        CanonicalCommand::ConvertGroupToComponent(plan) => {
            convert_group_to_component_model(&mut edit.product, plan)?;
        }
        CanonicalCommand::ApplySolidTool(plan) => {
            apply_solid_tool(&mut edit.product, plan)?;
        }
        _ => {}
    }
    Ok(())
}
