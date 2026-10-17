//! Features, their dimensions and parameters, and threads.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name,
            kind,
        } => {
            ensure_product_id(id.0)?;
            ensure_name(name)?;
            validate_feature_kind(kind, edit.product.tolerance.linear_mm())?;
            validate_topological_feature_context(edit.current.document_id(), *definition_id, kind)?;
            if let FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::PlanarFace { reference, .. },
                ..
            }) = kind
            {
                let evidence = edit
                    .current
                    .exact_reference_by_lineage(&reference.lineage_digest)
                    .filter(|evidence| *evidence == reference.as_ref())
                    .ok_or(CanonicalError::Sketch(
                        SketchError::InvalidPlanarFaceSupport,
                    ))?;
                edit.product
                    .exact_reference_evidence
                    .insert(reference.lineage_digest.clone(), Arc::new(evidence.clone()));
            }
            if edit.product.features.contains_key(id) {
                return Err(CanonicalError::FeatureAlreadyExists(*id));
            }
            let definition = edit
                .product
                .definitions
                .get(definition_id)
                .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
            let ownership = inferred_feature_body_ownership(&edit.product, definition, kind)?;
            let mut replacement = definition.as_ref().clone();
            replacement.feature_ids.push(*id);
            replacement.feature_body_ownership.insert(*id, ownership);
            edit.product
                .definitions
                .insert(*definition_id, Arc::new(replacement));
            edit.product.features.insert(
                *id,
                Arc::new(Feature {
                    id: *id,
                    definition_id: *definition_id,
                    name: name.clone(),
                    kind: kind.clone(),
                }),
            );
        }
        CanonicalCommand::DeleteFeature { id } => {
            let feature = edit
                .product
                .features
                .remove(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let definition = &edit.product.definitions[&feature.definition_id];
            let feature_ids = definition
                .feature_ids
                .iter()
                .cloned()
                .filter(|candidate| candidate != id)
                .collect();
            let mut feature_body_ownership = definition.feature_body_ownership.clone();
            feature_body_ownership.remove(id);
            for suppressed in edit.product.body_feature_suppression.values_mut() {
                suppressed.remove(id);
            }
            edit.product
                .body_feature_suppression
                .retain(|_, suppressed| !suppressed.is_empty());
            edit.product.definitions.insert(
                feature.definition_id,
                Arc::new(Definition {
                    feature_ids,
                    feature_body_ownership,
                    ..definition.as_ref().clone()
                }),
            );
            forget_feature_annotations(&mut edit.product, *id);
        }
        CanonicalCommand::SetFeatureDimension { id, dimension } => {
            let feature = edit
                .product
                .features
                .get(id)
                .ok_or(CanonicalError::FeatureNotFound(*id))?;
            let kind = match feature.kind {
                FeatureKind::Pad(ref spec) => {
                    let mut updated = spec.clone();
                    updated.extent = match spec.extent {
                        FeatureExtent::Blind(_) => FeatureExtent::Blind(dimension.clone()),
                        FeatureExtent::Symmetric(_) => FeatureExtent::Symmetric(dimension.clone()),
                        FeatureExtent::ThroughAll
                        | FeatureExtent::UpToFace(_)
                        | FeatureExtent::Bidirectional { .. } => {
                            return Err(CanonicalError::FeatureExtentHasNoSingleDistance(*id));
                        }
                    };
                    FeatureKind::Pad(updated)
                }
                FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Offset { base, .. },
                    ..
                }) => {
                    let base_frame = edit
                        .product
                        .features
                        .get(&base)
                        .and_then(|feature| match &feature.kind {
                            FeatureKind::Workplane(spec) => Some(spec.frame),
                            _ => None,
                        })
                        .ok_or(CanonicalError::Sketch(
                            SketchError::MissingWorkplaneSupport(base),
                        ))?;
                    FeatureKind::Workplane(WorkplaneSpec {
                        support: WorkplaneSupport::Offset {
                            base,
                            distance: dimension.clone(),
                        },
                        frame: base_frame.offset(dimension.millimetres()),
                    })
                }
                FeatureKind::Shell {
                    target,
                    ref removed_faces,
                    direction,
                    ..
                } => FeatureKind::Shell {
                    target,
                    removed_faces: removed_faces.clone(),
                    thickness: dimension.clone(),
                    direction,
                },
                FeatureKind::SurfaceThicken {
                    target, direction, ..
                } => FeatureKind::SurfaceThicken {
                    target,
                    thickness: dimension.clone(),
                    direction,
                },
                FeatureKind::EdgeFinish {
                    target,
                    ref edges,
                    kind,
                    ref fillet_radius_stations,
                    ref chamfer_mode,
                    ref chamfer_edge_sides,
                    ..
                } => FeatureKind::EdgeFinish {
                    target,
                    edges: edges.clone(),
                    kind,
                    amount: dimension.clone(),
                    fillet_radius_stations: fillet_radius_stations.clone(),
                    chamfer_mode: chamfer_mode.clone(),
                    chamfer_edge_sides: chamfer_edge_sides.clone(),
                },
                FeatureKind::PlanarOffset { profile, .. } => FeatureKind::PlanarOffset {
                    profile,
                    distance: dimension.clone(),
                },
                _ => return Err(CanonicalError::FeatureHasNoDimension(*id)),
            };
            edit.product.features.insert(
                *id,
                Arc::new(Feature {
                    id: *id,
                    definition_id: feature.definition_id,
                    name: feature.name.clone(),
                    kind,
                }),
            );
            recompute_offset_workplane_frames(&mut edit.product)?;
        }
        CanonicalCommand::SetFeatureParameter { target, dimension } => {
            set_feature_parameter(&mut edit.product, target, dimension.clone())?;
        }
        CanonicalCommand::SetHoleThread { .. }
        | CanonicalCommand::SetModeledThreads { .. }
        | CanonicalCommand::SetShaftThread { .. } => threads::apply(&mut edit.product, command)?,
        _ => return Err(CanonicalError::CommandOutsideFamily("feature")),
    }
    Ok(())
}
