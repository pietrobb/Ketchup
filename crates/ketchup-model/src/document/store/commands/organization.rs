//! Tags, saved views, underlays, annotations, classification, collections and import
//! receipts.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::CreateTag { id, name, visible } => {
            ensure_product_id(id.0)?;
            ensure_name(name)?;
            if edit.product.tags.contains_key(id) {
                return Err(CanonicalError::TagAlreadyExists(*id));
            }
            edit.product.tags.insert(
                *id,
                Arc::new(Tag {
                    id: *id,
                    name: name.clone(),
                    visible: *visible,
                }),
            );
        }
        CanonicalCommand::DeleteTag { id } => {
            ensure_tag_unassigned(&edit.product, *id)?;
            edit.product
                .tags
                .remove(id)
                .ok_or(CanonicalError::TagNotFound(*id))?;
            forget_tag_in_saved_views(&mut edit.product, *id);
        }
        CanonicalCommand::UpsertSavedView(_) | CanonicalCommand::DeleteSavedView { .. } => {
            apply_saved_view_command(&mut edit.product, command)?;
        }
        CanonicalCommand::UpsertUnderlay(underlay) => {
            ensure_product_id(underlay.id.0)?;
            validate_underlay(&edit.product, underlay)?;
            edit.product
                .underlays
                .insert(underlay.id, Arc::new(underlay.clone()));
        }
        CanonicalCommand::DeleteUnderlay { id } => {
            edit.product
                .underlays
                .remove(id)
                .ok_or(CanonicalError::UnderlayNotFound(*id))?;
        }
        CanonicalCommand::SetAnnotations { annotations } => {
            validate_annotations(&edit.product, annotations)?;
            edit.product.annotations = Arc::new(annotations.clone());
        }
        CanonicalCommand::SetTagVisibility { id, visible } => {
            let existing = edit
                .product
                .tags
                .get(id)
                .ok_or(CanonicalError::TagNotFound(*id))?;
            edit.product.tags.insert(
                *id,
                Arc::new(Tag {
                    visible: *visible,
                    ..existing.as_ref().clone()
                }),
            );
        }
        CanonicalCommand::SetTagName { id, name } => {
            ensure_name(name)?;
            let existing = edit
                .product
                .tags
                .get(id)
                .ok_or(CanonicalError::TagNotFound(*id))?;
            edit.product.tags.insert(
                *id,
                Arc::new(Tag {
                    name: name.clone(),
                    ..existing.as_ref().clone()
                }),
            );
        }
        CanonicalCommand::UpsertClassificationDimension {
            id,
            name,
            categories,
        } => {
            ensure_product_id(id.0)?;
            ensure_name(name)?;
            if categories.is_empty() || categories.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
                return Err(CanonicalError::InvalidClassificationDimension(*id));
            }
            let mut canonical_categories = BTreeMap::new();
            let mut names = BTreeSet::new();
            for (category_id, category_name) in categories {
                ensure_product_id(category_id.0)?;
                ensure_name(category_name)?;
                if !names.insert(category_name.clone()) {
                    return Err(CanonicalError::InvalidClassificationDimension(*id));
                }
                canonical_categories.insert(
                    *category_id,
                    ClassificationCategory {
                        id: *category_id,
                        name: category_name.clone(),
                    },
                );
            }
            if edit.product.classification_assignments.iter().any(
                |((_, dimension_id), category_id)| {
                    dimension_id == id && !canonical_categories.contains_key(category_id)
                },
            ) {
                return Err(CanonicalError::ClassificationCategoryInUse(*id));
            }
            edit.product.classification_dimensions.insert(
                *id,
                Arc::new(ClassificationDimension {
                    id: *id,
                    name: name.clone(),
                    categories: canonical_categories,
                }),
            );
        }
        CanonicalCommand::SetOccurrenceClassification {
            occurrence_id,
            dimension_id,
            category_id,
        } => {
            if !edit.product.occurrences.contains_key(occurrence_id) {
                return Err(CanonicalError::OccurrenceNotFound(*occurrence_id));
            }
            let dimension = edit
                .product
                .classification_dimensions
                .get(dimension_id)
                .ok_or(CanonicalError::ClassificationDimensionNotFound(
                    *dimension_id,
                ))?;
            if let Some(category_id) = category_id {
                if !dimension.categories.contains_key(category_id) {
                    return Err(CanonicalError::ClassificationCategoryNotFound(
                        *dimension_id,
                        *category_id,
                    ));
                }
                edit.product
                    .classification_assignments
                    .insert((*occurrence_id, *dimension_id), *category_id);
            } else {
                edit.product
                    .classification_assignments
                    .remove(&(*occurrence_id, *dimension_id));
            }
        }
        CanonicalCommand::CreateCollection { id, name } => {
            ensure_product_id(id.0)?;
            ensure_name(name)?;
            if edit.product.collections.contains_key(id) {
                return Err(CanonicalError::CollectionAlreadyExists(*id));
            }
            edit.product.collections.insert(
                *id,
                Arc::new(Collection {
                    id: *id,
                    name: name.clone(),
                    occurrence_ids: BTreeSet::new(),
                }),
            );
        }
        CanonicalCommand::DeleteCollection { id } => {
            edit.product
                .collections
                .remove(id)
                .ok_or(CanonicalError::CollectionNotFound(*id))?;
        }
        CanonicalCommand::SetCollectionOccurrences { id, occurrence_ids } => {
            let existing = edit
                .product
                .collections
                .get(id)
                .ok_or(CanonicalError::CollectionNotFound(*id))?;
            if occurrence_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(CanonicalError::CollectionMembershipNotCanonical(*id));
            }
            let canonical = occurrence_ids.iter().cloned().collect::<BTreeSet<_>>();
            for occurrence_id in &canonical {
                if !edit.product.occurrences.contains_key(occurrence_id) {
                    return Err(CanonicalError::OccurrenceNotFound(*occurrence_id));
                }
            }
            edit.product.collections.insert(
                *id,
                Arc::new(Collection {
                    occurrence_ids: canonical,
                    ..existing.as_ref().clone()
                }),
            );
        }
        CanonicalCommand::RecordImport(receipt) => {
            receipt
                .validate()
                .map_err(CanonicalError::InvalidImportReceipt)?;
            if edit.product.import_receipts.contains_key(&receipt.id()) {
                return Err(CanonicalError::ImportAlreadyExists(receipt.id()));
            }
            for output in receipt.outputs() {
                let missing = match output {
                    ImportOutputRef::Definition(id) => (!edit.product.definitions.contains_key(id))
                        .then_some(CanonicalError::DefinitionNotFound(*id)),
                    ImportOutputRef::Feature(id) => (!edit.product.features.contains_key(id))
                        .then_some(CanonicalError::FeatureNotFound(*id)),
                    ImportOutputRef::Occurrence(id) => (!edit.product.occurrences.contains_key(id))
                        .then_some(CanonicalError::OccurrenceNotFound(*id)),
                    ImportOutputRef::Group(id) => (!edit.product.groups.contains_key(id))
                        .then_some(CanonicalError::GroupNotFound(*id)),
                };
                if let Some(error) = missing {
                    return Err(error);
                }
            }
            edit.product
                .import_receipts
                .insert(receipt.id(), Arc::new(receipt.clone()));
        }
        _ => return Err(CanonicalError::CommandOutsideFamily("organization")),
    }
    Ok(())
}
