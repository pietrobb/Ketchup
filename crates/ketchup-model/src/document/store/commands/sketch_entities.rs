//! Sketch entity edits and profile points.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::SplitSketchEntity {
            id,
            entity_id,
            new_entity_id,
            parameter,
            joint_constraint_ids,
        } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.split_entity(*entity_id, *new_entity_id, *parameter, joint_constraint_ids)?;
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
        CanonicalCommand::JoinSketchEntities {
            id,
            source_entity_id,
            source_endpoint,
            consumed_entity_id,
            consumed_endpoint,
        } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.join_entities(
                *source_entity_id,
                *source_endpoint,
                *consumed_entity_id,
                *consumed_endpoint,
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
        CanonicalCommand::TrimSketchEntity {
            id,
            entity_id,
            start_parameter,
            end_parameter,
        } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.trim_entity(*entity_id, *start_parameter, *end_parameter)?;
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
        CanonicalCommand::ExtendSketchEntity {
            id,
            entity_id,
            endpoint,
            parameter,
        } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.extend_entity(*entity_id, *endpoint, *parameter)?;
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
        CanonicalCommand::OffsetSketchEntity {
            id,
            entity_id,
            new_entity_id,
            distance_mm,
            side,
        } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.offset_entity(*entity_id, *new_entity_id, *distance_mm, *side)?;
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
        CanonicalCommand::ProjectSketchEntity {
            id,
            source_feature_id,
            source_entity_id,
            new_entity_id,
            projection_constraint_id,
        } => {
            let projected = project_sketch_entity(
                &edit.product,
                *id,
                *source_feature_id,
                *source_entity_id,
                *new_entity_id,
            )?;
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.add_projection(
                projected,
                *source_feature_id,
                *source_entity_id,
                *projection_constraint_id,
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
        CanonicalCommand::SetSketchEntityConstruction {
            id,
            entity_id,
            construction,
            construction_constraint_id,
        } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let FeatureKind::Sketch(spec) = &feature.kind else {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            };
            let mut updated = spec.clone();
            updated.set_entity_construction(
                *entity_id,
                *construction,
                *construction_constraint_id,
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
        CanonicalCommand::TranslateProfile { id, delta_mm } => {
            if !delta_mm.iter().all(|value| value.is_finite())
                || (delta_mm[0] == 0.0 && delta_mm[1] == 0.0)
            {
                return Err(CanonicalError::InvalidProfile);
            }
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let translate = |point: &mut [f64; 2]| {
                point[0] += delta_mm[0];
                point[1] += delta_mm[1];
            };
            let mut kind = feature.kind.clone();
            match &mut kind {
                FeatureKind::Profile { segments, .. } => {
                    segments
                        .iter_mut()
                        .flat_map(ProfileSegment::defining_points_mut)
                        .for_each(translate);
                }
                FeatureKind::Sketch(spec) => {
                    if spec
                        .entities
                        .iter()
                        .any(|entity| spec.is_projected_entity(entity.id()))
                    {
                        return Err(CanonicalError::Sketch(SketchError::InvalidProjectionSource));
                    }
                    for entity in &mut spec.entities {
                        match entity {
                            SketchEntity::Line {
                                start_mm, end_mm, ..
                            } => {
                                translate(start_mm);
                                translate(end_mm);
                            }
                            SketchEntity::Arc {
                                start_mm,
                                end_mm,
                                center_mm,
                                ..
                            } => {
                                translate(start_mm);
                                translate(end_mm);
                                translate(center_mm);
                            }
                            SketchEntity::Circle { center_mm, .. } => translate(center_mm),
                            SketchEntity::CubicBezier {
                                start_mm,
                                control_1_mm,
                                control_2_mm,
                                end_mm,
                                ..
                            } => {
                                translate(start_mm);
                                translate(control_1_mm);
                                translate(control_2_mm);
                                translate(end_mm);
                            }
                        }
                    }
                    for constraint in &mut spec.constraints {
                        if let SketchConstraintKind::FixedPoint { position_mm, .. } =
                            &mut constraint.kind
                        {
                            translate(position_mm);
                        }
                    }
                }
                _ => return Err(CanonicalError::FeatureIsNotProfile(*id)),
            }
            validate_feature_kind(&kind, edit.product.tolerance.linear_mm())?;
            edit.product.features.insert(
                *id,
                Arc::new(Feature {
                    id: *id,
                    definition_id: feature.definition_id,
                    name: feature.name.clone(),
                    kind,
                }),
            );
        }
        CanonicalCommand::SetProfilePoints { id, points_mm } => {
            let kind = FeatureKind::polygon(points_mm);
            validate_feature_kind(&kind, edit.product.tolerance.linear_mm())?;
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            if feature.kind.polygon_points().is_none() {
                return Err(CanonicalError::FeatureIsNotProfile(*id));
            }
            edit.product.features.insert(
                *id,
                Arc::new(Feature {
                    id: *id,
                    definition_id: feature.definition_id,
                    name: feature.name.clone(),
                    kind,
                }),
            );
        }
        _ => return Err(CanonicalError::CommandOutsideFamily("sketch entity")),
    }
    Ok(())
}
