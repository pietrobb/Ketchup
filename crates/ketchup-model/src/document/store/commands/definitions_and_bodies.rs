//! Definitions and the bodies they hold.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::CreateDefinition { id, name } => {
            ensure_product_id(id.0)?;
            ensure_name(name)?;
            if edit.product.definitions.contains_key(id) {
                return Err(CanonicalError::DefinitionAlreadyExists(*id));
            }
            edit.product
                .definitions
                .insert(*id, Arc::new(new_definition(*id, name.clone())));
        }
        CanonicalCommand::DeleteDefinition { id } => {
            if edit
                .product
                .occurrences
                .values()
                .any(|occurrence| occurrence.definition_id == *id)
            {
                return Err(CanonicalError::DefinitionInUse(*id));
            }
            let definition = edit
                .product
                .definitions
                .remove(id)
                .ok_or(CanonicalError::DefinitionNotFound(*id))?;
            edit.product
                .body_feature_suppression
                .retain(|(definition_id, _), _| definition_id != id);
            for feature_id in &definition.feature_ids {
                edit.product.features.remove(feature_id);
                forget_feature_annotations(&mut edit.product, *feature_id);
            }
        }
        CanonicalCommand::RenameDefinition { id, name } => {
            ensure_name(name)?;
            let existing = edit
                .product
                .definitions
                .get(id)
                .ok_or(CanonicalError::DefinitionNotFound(*id))?;
            edit.product.definitions.insert(
                *id,
                Arc::new(Definition {
                    name: name.clone(),
                    ..existing.as_ref().clone()
                }),
            );
        }
        CanonicalCommand::CreateBody {
            definition_id,
            id,
            name,
            visible,
        } => {
            ensure_product_id(id.0)?;
            ensure_name(name)?;
            let definition = edit
                .product
                .definitions
                .get(definition_id)
                .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
            if definition.bodies.contains_key(id) {
                return Err(CanonicalError::BodyAlreadyExists(*definition_id, *id));
            }
            let mut replacement = definition.as_ref().clone();
            replacement.bodies.insert(
                *id,
                Body {
                    id: *id,
                    name: name.clone(),
                    visible: *visible,
                    consumed_by: None,
                },
            );
            edit.product
                .definitions
                .insert(*definition_id, Arc::new(replacement));
        }
        CanonicalCommand::DeleteBody { definition_id, id } => {
            let definition = edit
                .product
                .definitions
                .get(definition_id)
                .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
            if !definition.bodies.contains_key(id) {
                return Err(CanonicalError::BodyNotFound(*definition_id, *id));
            }
            if definition.active_body_id == *id {
                return Err(CanonicalError::BodyIsActive(*definition_id, *id));
            }
            if definition.feature_body_ownership.values().any(|ownership| {
                ownership.output_body_id == Some(*id) || ownership.input_body_ids.contains(id)
            }) {
                return Err(CanonicalError::BodyInUse(*definition_id, *id));
            }
            let mut replacement = definition.as_ref().clone();
            replacement.bodies.remove(id);
            edit.product
                .definitions
                .insert(*definition_id, Arc::new(replacement));
        }
        CanonicalCommand::RenameBody {
            definition_id,
            id,
            name,
        } => {
            ensure_name(name)?;
            let definition = edit
                .product
                .definitions
                .get(definition_id)
                .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
            let body = definition
                .bodies
                .get(id)
                .ok_or(CanonicalError::BodyNotFound(*definition_id, *id))?;
            let mut replacement = definition.as_ref().clone();
            replacement.bodies.insert(
                *id,
                Body {
                    name: name.clone(),
                    ..body.clone()
                },
            );
            edit.product
                .definitions
                .insert(*definition_id, Arc::new(replacement));
        }
        CanonicalCommand::SetActiveBody { definition_id, id } => {
            let definition = edit
                .product
                .definitions
                .get(definition_id)
                .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
            let body = definition
                .bodies
                .get(id)
                .ok_or(CanonicalError::BodyNotFound(*definition_id, *id))?;
            if body.consumed_by.is_some() {
                return Err(CanonicalError::InvalidBodyAuthoringPlan);
            }
            let mut replacement = definition.as_ref().clone();
            replacement.active_body_id = *id;
            edit.product
                .definitions
                .insert(*definition_id, Arc::new(replacement));
        }
        CanonicalCommand::SetBodyVisibility {
            definition_id,
            id,
            visible,
        } => {
            let definition = edit
                .product
                .definitions
                .get(definition_id)
                .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
            let body = definition
                .bodies
                .get(id)
                .ok_or(CanonicalError::BodyNotFound(*definition_id, *id))?;
            let mut replacement = definition.as_ref().clone();
            replacement.bodies.insert(
                *id,
                Body {
                    visible: *visible,
                    ..body.clone()
                },
            );
            edit.product
                .definitions
                .insert(*definition_id, Arc::new(replacement));
        }
        CanonicalCommand::ConsumeBody {
            definition_id,
            id,
            by_feature_id,
        } => {
            let definition = edit
                .product
                .definitions
                .get(definition_id)
                .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
            let body = definition
                .bodies
                .get(id)
                .ok_or(CanonicalError::BodyNotFound(*definition_id, *id))?;
            let feature = edit
                .product
                .features
                .get(by_feature_id)
                .ok_or(CanonicalError::FeatureNotFound(*by_feature_id))?;
            let ownership = definition
                .feature_body_ownership
                .get(by_feature_id)
                .ok_or(CanonicalError::InvalidBodyAuthoringPlan)?;
            if body.consumed_by.is_some()
                || definition.active_body_id == *id
                || feature.definition_id != *definition_id
                || !matches!(feature.kind, FeatureKind::Boolean { .. })
                || !ownership.input_body_ids.contains(id)
                || ownership.output_body_id == Some(*id)
            {
                return Err(CanonicalError::InvalidBodyAuthoringPlan);
            }
            let mut replacement = definition.as_ref().clone();
            replacement.bodies.insert(
                *id,
                Body {
                    consumed_by: Some(*by_feature_id),
                    ..body.clone()
                },
            );
            edit.product
                .definitions
                .insert(*definition_id, Arc::new(replacement));
        }
        CanonicalCommand::SetFeatureBodyOwnership { id, ownership } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            validate_feature_body_ownership_change(&edit.product, feature, ownership)?;
            let definition = &edit.product.definitions[&feature.definition_id];
            let mut replacement = definition.as_ref().clone();
            replacement
                .feature_body_ownership
                .insert(*id, ownership.clone());
            edit.product
                .definitions
                .insert(feature.definition_id, Arc::new(replacement));
        }
        CanonicalCommand::SetBodyFeatureSuppression {
            definition_id,
            body_id,
            suppressed_feature_ids,
        } => {
            let graph = FeatureDependencyGraph::from_product(&edit.product)?;
            validate_body_feature_suppression(
                &edit.product,
                *definition_id,
                *body_id,
                suppressed_feature_ids,
                &graph,
            )?;
            let key = (*definition_id, *body_id);
            let current_suppressed = edit
                .product
                .body_feature_suppression
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let requested_suppressed = suppressed_feature_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            if current_suppressed == requested_suppressed {
                return Err(CanonicalError::FeatureSuppressionUnchanged(
                    *definition_id,
                    *body_id,
                ));
            }
            edit.explicit_dirty_features.extend(
                current_suppressed
                    .iter()
                    .chain(&requested_suppressed)
                    .cloned(),
            );
            if requested_suppressed.is_empty() {
                edit.product.body_feature_suppression.remove(&key);
            } else {
                edit.product
                    .body_feature_suppression
                    .insert(key, requested_suppressed);
            }
        }
        _ => {}
    }
    Ok(())
}
