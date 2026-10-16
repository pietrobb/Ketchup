//! Sketch constraints and their dimensions.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::CreateSketchConstraint { id, constraint } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.create_constraint(constraint.clone())?;
            validate_sketch_constraint_edit_dependents(
                &edit.product,
                *id,
                &updated,
                constraint.id,
            )?;
            edit.product.features.insert(
                *id,
                Arc::new(Feature {
                    id: *id,
                    definition_id: feature.definition_id,
                    name: feature.name.clone(),
                    kind: FeatureKind::Sketch(updated),
                }),
            );
        }
        CanonicalCommand::ReplaceSketchConstraint { id, constraint } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.replace_constraint(constraint.clone())?;
            validate_sketch_constraint_edit_dependents(
                &edit.product,
                *id,
                &updated,
                constraint.id,
            )?;
            edit.product.features.insert(
                *id,
                Arc::new(Feature {
                    id: *id,
                    definition_id: feature.definition_id,
                    name: feature.name.clone(),
                    kind: FeatureKind::Sketch(updated),
                }),
            );
        }
        CanonicalCommand::DeleteSketchConstraint { id, constraint_id } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.delete_constraint(*constraint_id)?;
            validate_sketch_constraint_edit_dependents(
                &edit.product,
                *id,
                &updated,
                *constraint_id,
            )?;
            edit.product.features.insert(
                *id,
                Arc::new(Feature {
                    id: *id,
                    definition_id: feature.definition_id,
                    name: feature.name.clone(),
                    kind: FeatureKind::Sketch(updated),
                }),
            );
        }
        CanonicalCommand::SetSketchConstraintDimension {
            id,
            constraint_id,
            dimension,
        } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureHasNoDimension(*id));
            };
            let mut updated = spec.clone();
            let constraint = updated
                .constraints
                .iter_mut()
                .find(|constraint| constraint.id == *constraint_id)
                .ok_or(CanonicalError::Sketch(
                    SketchError::InvalidConstraintReference(*constraint_id),
                ))?;
            match &mut constraint.kind {
                SketchConstraintKind::Distance { value, .. }
                | SketchConstraintKind::Radius { value, .. } => {
                    *value = dimension.clone();
                }
                SketchConstraintKind::Angle { angle_degrees, .. } => {
                    *angle_degrees = dimension.millimetres();
                }
                _ => return Err(CanonicalError::FeatureHasNoDimension(*id)),
            }
            edit.product.features.insert(
                *id,
                Arc::new(Feature {
                    id: *id,
                    definition_id: feature.definition_id,
                    name: feature.name.clone(),
                    kind: FeatureKind::Sketch(updated),
                }),
            );
        }
        _ => return Err(CanonicalError::CommandOutsideFamily("sketch constraint")),
    }
    Ok(())
}
