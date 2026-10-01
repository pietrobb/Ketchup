use super::*;

pub(super) fn remap_body_subshape_reference(
    reference: &BodySubshapeRef,
    new_definition_id: DefinitionId,
    mapping: &BTreeMap<FeatureId, FeatureId>,
) -> Result<BodySubshapeRef, CanonicalError> {
    let mut remapped = reference.clone();
    remapped.definition_id = new_definition_id;
    remapped.profile_feature_id = *mapping
        .get(&reference.profile_feature_id)
        .ok_or(CanonicalError::InvalidFeatureMap)?;
    remapped.producer_feature_id = *mapping
        .get(&reference.producer_feature_id)
        .ok_or(CanonicalError::InvalidFeatureMap)?;
    remapped.lineage_digest = canonical_reference_lineage_digest(
        remapped.document_id,
        remapped.producer_feature_id,
        &remapped.semantic_role,
        &remapped.source_element_id,
        &remapped.expected_type,
    );
    Ok(remapped)
}

pub(super) fn remap_feature_extent(
    extent: &mut FeatureExtent,
    new_definition_id: DefinitionId,
    mapping: &BTreeMap<FeatureId, FeatureId>,
) -> Result<(), CanonicalError> {
    match extent {
        FeatureExtent::UpToFace(reference) => {
            **reference = remap_body_subshape_reference(reference, new_definition_id, mapping)?;
        }
        FeatureExtent::Bidirectional { along, opposite } => {
            for end in [along, opposite] {
                if let FeatureExtentEnd::UpToFace(reference) = end {
                    **reference =
                        remap_body_subshape_reference(reference, new_definition_id, mapping)?;
                }
            }
        }
        FeatureExtent::Blind(_) | FeatureExtent::ThroughAll | FeatureExtent::Symmetric(_) => {}
    }
    Ok(())
}

/// Named faces are relative to their target and carry over unchanged.
pub(super) fn remap_face_ref(
    face: &FaceRef,
    new_definition_id: DefinitionId,
    mapping: &BTreeMap<FeatureId, FeatureId>,
) -> Result<FaceRef, CanonicalError> {
    Ok(match face {
        FaceRef::Topological(reference) => FaceRef::from(remap_topological_reference(
            reference,
            new_definition_id,
            mapping,
        )?),
        FaceRef::Named(_) => face.clone(),
    })
}

pub(super) fn remap_edge_ref(
    edge: &EdgeRef,
    new_definition_id: DefinitionId,
    mapping: &BTreeMap<FeatureId, FeatureId>,
) -> Result<EdgeRef, CanonicalError> {
    Ok(match edge {
        EdgeRef::Topological(reference) => EdgeRef::from(remap_topological_reference(
            reference,
            new_definition_id,
            mapping,
        )?),
        EdgeRef::Named(_) => edge.clone(),
    })
}

pub(super) fn remap_topological_reference(
    reference: &TopologicalElementRef,
    new_definition_id: DefinitionId,
    mapping: &BTreeMap<FeatureId, FeatureId>,
) -> Result<TopologicalElementRef, CanonicalError> {
    TopologicalElementRef::new(
        reference.document_id,
        new_definition_id,
        *mapping
            .get(&reference.source_feature_id)
            .ok_or(CanonicalError::InvalidFeatureMap)?,
        *mapping
            .get(&reference.producer_feature_id)
            .ok_or(CanonicalError::InvalidFeatureMap)?,
        reference.kind,
        reference.source_element_id.clone(),
        reference.producer_element_id.clone(),
        reference.stability,
        reference.evaluator.clone(),
        reference.backend.clone(),
        reference.tolerance.clone(),
        reference.result_fingerprint.clone(),
        reference.corroborating_geometry_fingerprint.clone(),
    )
    .map_err(|error| CanonicalError::InvalidFeatureMap.because(error))
}

pub(super) fn remap_loft_sections(
    sections: &[LoftSection],
    mapping: &BTreeMap<FeatureId, FeatureId>,
) -> Result<Vec<LoftSection>, CanonicalError> {
    sections
        .iter()
        .map(|section| {
            Ok(LoftSection {
                profile: *mapping
                    .get(&section.profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                elevation_mm: section.elevation_mm,
            })
        })
        .collect()
}

pub(super) fn remap_optional_feature(
    feature: Option<FeatureId>,
    mapping: &BTreeMap<FeatureId, FeatureId>,
) -> Result<Option<FeatureId>, CanonicalError> {
    feature
        .map(|feature| {
            mapping
                .get(&feature)
                .copied()
                .ok_or(CanonicalError::InvalidFeatureMap)
        })
        .transpose()
}

pub(super) fn clone_definition_and_repoint(
    product: &mut ProductModel,
    plan: &CloneDefinitionPlan,
) -> Result<(), CanonicalError> {
    let occurrence_id = plan.occurrence_id;
    let source_definition_id = plan.source_definition_id;
    let new_definition_id = plan.new_definition_id;
    let new_definition_name = &plan.new_definition_name;
    let feature_id_map = &plan.feature_id_map;
    ensure_product_id(new_definition_id.0)?;
    ensure_name(new_definition_name)?;
    if product.definitions.contains_key(&new_definition_id) {
        return Err(CanonicalError::DefinitionAlreadyExists(new_definition_id));
    }
    let occurrence = product
        .occurrences
        .get(&occurrence_id)
        .ok_or(CanonicalError::OccurrenceNotFound(occurrence_id))?
        .as_ref()
        .clone();
    if occurrence.definition_id != source_definition_id {
        return Err(CanonicalError::OccurrenceDefinitionMismatch);
    }
    let source = product
        .definitions
        .get(&source_definition_id)
        .ok_or(CanonicalError::DefinitionNotFound(source_definition_id))?
        .as_ref()
        .clone();
    if feature_id_map.len() != source.feature_ids.len()
        || feature_id_map
            .iter()
            .map(|(source_id, _)| *source_id)
            .ne(source.feature_ids.iter().cloned())
    {
        return Err(CanonicalError::InvalidFeatureMap);
    }
    let mut mapped_ids = BTreeSet::new();
    let mut mapping = BTreeMap::new();
    for (source_id, new_id) in feature_id_map {
        ensure_product_id(new_id.0)?;
        if !mapped_ids.insert(*new_id) || product.features.contains_key(new_id) {
            return Err(CanonicalError::InvalidFeatureMap);
        }
        mapping.insert(*source_id, *new_id);
    }

    let mut cloned_features = Vec::with_capacity(feature_id_map.len());
    for (source_id, new_id) in feature_id_map {
        let source_feature = product
            .features
            .get(source_id)
            .ok_or(CanonicalError::FeatureNotFound(*source_id))?;
        let kind = match &source_feature.kind {
            FeatureKind::Workplane(spec) => {
                let mut cloned = spec.clone();
                match &mut cloned.support {
                    WorkplaneSupport::Free | WorkplaneSupport::Principal(_) => {}
                    WorkplaneSupport::Offset { base, .. }
                    | WorkplaneSupport::ConstructionPlane { feature: base } => {
                        *base = *mapping.get(base).ok_or(CanonicalError::InvalidFeatureMap)?;
                    }
                    WorkplaneSupport::PlanarFace { reference, .. } => {
                        **reference =
                            remap_body_subshape_reference(reference, new_definition_id, &mapping)?;
                    }
                }
                FeatureKind::Workplane(cloned)
            }
            FeatureKind::Sketch(spec) => {
                let mut cloned = spec.clone();
                cloned.workplane = *mapping
                    .get(&spec.workplane)
                    .ok_or(CanonicalError::InvalidFeatureMap)?;
                FeatureKind::Sketch(cloned)
            }
            FeatureKind::Profile { segments, closed } => FeatureKind::Profile {
                segments: segments.clone(),
                closed: *closed,
            },
            FeatureKind::SpatialPath { segments } => FeatureKind::SpatialPath {
                segments: segments.clone(),
            },
            FeatureKind::ConstructionPoint { position_mm } => FeatureKind::ConstructionPoint {
                position_mm: *position_mm,
            },
            FeatureKind::ConstructionAxis {
                origin_mm,
                direction,
            } => FeatureKind::ConstructionAxis {
                origin_mm: *origin_mm,
                direction: *direction,
            },
            FeatureKind::ConstructionPlane {
                origin_mm,
                normal,
                x_direction,
            } => FeatureKind::ConstructionPlane {
                origin_mm: *origin_mm,
                normal: *normal,
                x_direction: *x_direction,
            },
            FeatureKind::Pad(spec) => {
                let mut cloned = spec.with_features(|id| {
                    mapping
                        .get(&id)
                        .copied()
                        .ok_or(CanonicalError::InvalidFeatureMap)
                })?;
                if let PadOperation::Cut {
                    start: CutStart::Support(support),
                    ..
                } = &mut cloned.operation
                {
                    **support =
                        remap_body_subshape_reference(support, new_definition_id, &mapping)?;
                }
                remap_feature_extent(&mut cloned.extent, new_definition_id, &mapping)?;
                FeatureKind::Pad(cloned)
            }
            FeatureKind::Revolve {
                profile,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => FeatureKind::Revolve {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                axis_start_mm: *axis_start_mm,
                axis_end_mm: *axis_end_mm,
                angle_degrees: *angle_degrees,
            },
            FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
                direction,
            } => FeatureKind::Shell {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                removed_faces: removed_faces
                    .iter()
                    .map(|face| remap_face_ref(face, new_definition_id, &mapping))
                    .collect::<Result<Vec<_>, _>>()?,
                thickness: thickness.clone(),
                direction: *direction,
            },
            FeatureKind::EdgeFinish {
                target,
                edges,
                kind,
                amount,
                fillet_radius_stations,
                chamfer_mode,
                chamfer_edge_sides,
            } => FeatureKind::EdgeFinish {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                edges: edges
                    .iter()
                    .map(|edge| remap_edge_ref(edge, new_definition_id, &mapping))
                    .collect::<Result<Vec<_>, _>>()?,
                kind: *kind,
                amount: amount.clone(),
                fillet_radius_stations: fillet_radius_stations.clone(),
                chamfer_mode: chamfer_mode.clone(),
                chamfer_edge_sides: chamfer_edge_sides
                    .iter()
                    .map(|selection| {
                        Ok(ChamferEdgeSide {
                            edge: remap_topological_reference(
                                &selection.edge,
                                new_definition_id,
                                &mapping,
                            )?,
                            side_face: remap_topological_reference(
                                &selection.side_face,
                                new_definition_id,
                                &mapping,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, CanonicalError>>()?,
            },
            FeatureKind::FaceOffset {
                target,
                face,
                distance,
            } => FeatureKind::FaceOffset {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                face: remap_face_ref(face, new_definition_id, &mapping)?,
                distance: distance.clone(),
            },
            FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => FeatureKind::Boolean {
                operation: *operation,
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                tool: *mapping.get(tool).ok_or(CanonicalError::InvalidFeatureMap)?,
            },
            FeatureKind::PlanarOffset { profile, distance } => FeatureKind::PlanarOffset {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                distance: distance.clone(),
            },
            FeatureKind::Sweep { profile, path } => FeatureKind::Sweep {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                path: *mapping.get(path).ok_or(CanonicalError::InvalidFeatureMap)?,
            },
            FeatureKind::WeldmentMember(spec) => FeatureKind::WeldmentMember(WeldmentMemberSpec {
                profile: *mapping
                    .get(&spec.profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                path: *mapping
                    .get(&spec.path)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                orientation_degrees: spec.orientation_degrees,
            }),
            FeatureKind::WeldmentJoint(spec) => FeatureKind::WeldmentJoint(WeldmentJointSpec {
                first_member: *mapping
                    .get(&spec.first_member)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                second_member: *mapping
                    .get(&spec.second_member)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                policy: spec.policy,
                primary: spec.primary,
            }),
            FeatureKind::SurfaceBody(spec) => FeatureKind::SurfaceBody(match spec {
                SurfaceBodySpec::Planar { profile } => SurfaceBodySpec::Planar {
                    profile: *mapping
                        .get(profile)
                        .ok_or(CanonicalError::InvalidFeatureMap)?,
                },
                SurfaceBodySpec::Loft {
                    sections,
                    guide,
                    continuity,
                } => SurfaceBodySpec::Loft {
                    sections: remap_loft_sections(sections, &mapping)?,
                    guide: remap_optional_feature(*guide, &mapping)?,
                    continuity: *continuity,
                },
            }),
            FeatureKind::SurfaceTrim { target, cutter } => FeatureKind::SurfaceTrim {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                cutter: *mapping
                    .get(cutter)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
            },
            FeatureKind::SurfaceExtend { target, distance } => FeatureKind::SurfaceExtend {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                distance: distance.clone(),
            },
            FeatureKind::SurfaceThicken {
                target,
                thickness,
                direction,
            } => FeatureKind::SurfaceThicken {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                thickness: thickness.clone(),
                direction: *direction,
            },
            FeatureKind::SurfaceKnit {
                surfaces,
                tolerance,
                make_solid,
            } => FeatureKind::SurfaceKnit {
                surfaces: surfaces
                    .iter()
                    .map(|surface| {
                        mapping
                            .get(surface)
                            .copied()
                            .ok_or(CanonicalError::InvalidFeatureMap)
                    })
                    .collect::<Result<_, _>>()?,
                tolerance: tolerance.clone(),
                make_solid: *make_solid,
            },
            FeatureKind::Loft {
                sections,
                guide,
                continuity,
            } => FeatureKind::Loft {
                sections: remap_loft_sections(sections, &mapping)?,
                guide: remap_optional_feature(*guide, &mapping)?,
                continuity: *continuity,
            },
            FeatureKind::SheetMetal(spec) => FeatureKind::SheetMetal(spec.clone()),
            FeatureKind::ImportedExactBody(spec) => FeatureKind::ImportedExactBody(spec.clone()),
            FeatureKind::RigidTransform { target, transform } => FeatureKind::RigidTransform {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                transform: *transform,
            },
            FeatureKind::MeshBody(spec) => {
                let mut spec = spec.clone();
                if let MeshAuthority::ExactConversion(conversion) = &mut spec.authority {
                    conversion.destination_definition_id = new_definition_id;
                    conversion.destination_feature_id = *new_id;
                }
                FeatureKind::MeshBody(spec)
            }
        };
        cloned_features.push(Arc::new(Feature {
            id: *new_id,
            definition_id: new_definition_id,
            name: source_feature.name.clone(),
            kind,
        }));
    }

    let feature_body_ownership = source
        .feature_body_ownership
        .iter()
        .map(|(source_id, ownership)| {
            Ok((
                *mapping
                    .get(source_id)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                ownership.clone(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CanonicalError>>()?;
    let bodies = source
        .bodies
        .iter()
        .map(|(id, body)| {
            Ok((
                *id,
                Body {
                    consumed_by: body
                        .consumed_by
                        .map(|feature_id| {
                            mapping
                                .get(&feature_id)
                                .cloned()
                                .ok_or(CanonicalError::InvalidFeatureMap)
                        })
                        .transpose()?,
                    ..body.clone()
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CanonicalError>>()?;
    product.definitions.insert(
        new_definition_id,
        Arc::new(Definition {
            id: new_definition_id,
            name: new_definition_name.to_owned(),
            feature_ids: feature_id_map.iter().map(|(_, new_id)| *new_id).collect(),
            bodies,
            active_body_id: source.active_body_id,
            feature_body_ownership,
            local_occurrence_ids: source.local_occurrence_ids.clone(),
            local_group_ids: source.local_group_ids.clone(),
        }),
    );
    for local_id in &source.local_group_ids {
        let old_key = LocalGroupKey {
            definition_id: source_definition_id,
            local_id: *local_id,
        };
        let local = product.local_groups[&old_key].as_ref();
        let key = LocalGroupKey {
            definition_id: new_definition_id,
            local_id: *local_id,
        };
        product.local_groups.insert(
            key,
            Arc::new(LocalGroup {
                key,
                name: local.name.clone(),
                transform: local.transform,
                parent: local.parent,
            }),
        );
    }
    for local_id in &source.local_occurrence_ids {
        let old_key = LocalOccurrenceKey {
            definition_id: source_definition_id,
            local_id: *local_id,
        };
        let local = product.local_occurrences[&old_key].as_ref();
        let key = LocalOccurrenceKey {
            definition_id: new_definition_id,
            local_id: *local_id,
        };
        product.local_occurrences.insert(
            key,
            Arc::new(LocalOccurrence {
                key,
                definition_id: local.definition_id,
                name: local.name.clone(),
                transform: local.transform,
                parent: local.parent,
                tag: local.tag,
                visible: local.visible,
                color: local.color,
            }),
        );
    }
    for feature in cloned_features {
        product.features.insert(feature.id, feature);
    }
    for ((_, body_id), suppressed) in product
        .body_feature_suppression
        .clone()
        .into_iter()
        .filter(|((definition_id, _), _)| *definition_id == source_definition_id)
    {
        let mapped = suppressed
            .into_iter()
            .map(|feature_id| {
                mapping
                    .get(&feature_id)
                    .cloned()
                    .ok_or(CanonicalError::InvalidFeatureMap)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        product
            .body_feature_suppression
            .insert((new_definition_id, body_id), mapped);
    }
    let cloned_bindings = product
        .feature_parameter_bindings
        .values()
        .filter_map(|binding| {
            mapping
                .get(&binding.target.feature_id)
                .map(|new_feature_id| {
                    let target = FeatureParameterTarget {
                        feature_id: *new_feature_id,
                        path: binding.target.path.clone(),
                        value_type: binding.target.value_type,
                    };
                    (
                        target.clone(),
                        Arc::new(FeatureParameterBinding {
                            target,
                            derived_from: binding.derived_from.clone(),
                        }),
                    )
                })
        })
        .collect::<Vec<_>>();
    product.feature_parameter_bindings.extend(cloned_bindings);
    product.occurrences.insert(
        occurrence_id,
        Arc::new(Occurrence {
            definition_id: new_definition_id,
            ..occurrence
        }),
    );
    let path = InstancePath::root(occurrence_id);
    for joint in product.pin_joints.values_mut() {
        let first = joint.first.instance_path == path;
        let second = joint.second.instance_path == path;
        if (first || second) && joint.physical_hole_pairs.is_some() {
            for pair in Arc::make_mut(joint).physical_hole_pairs.as_mut().unwrap() {
                if first {
                    pair.first_pocket_feature_id = *mapping
                        .get(&pair.first_pocket_feature_id)
                        .ok_or(CanonicalError::InvalidFeatureMap)?;
                }
                if second {
                    pair.second_pocket_feature_id = *mapping
                        .get(&pair.second_pocket_feature_id)
                        .ok_or(CanonicalError::InvalidFeatureMap)?;
                }
            }
        }
    }
    if let Some(recipe) = &mut product.assembly_recipe {
        let recipe = Arc::make_mut(recipe);
        for part in recipe
            .parts
            .values_mut()
            .filter(|part| part.instance_path == path)
        {
            if part.definition_id != source_definition_id {
                return Err(CanonicalError::AssemblyRecipe(
                    AssemblyRecipeError::PartChanged(part.key.clone()),
                ));
            }
            part.definition_id = new_definition_id;
            part.edit_scope = crate::assembly_recipe::RecipeEditScope::Occurrence(path.clone());
            for parameter in part.parameters.values_mut() {
                if let Some(target) = &mut parameter.target
                    && let Some(mapped) = mapping.get(&target.feature_id)
                {
                    target.feature_id = *mapped;
                }
            }
            for owned in recipe
                .owned_features
                .values_mut()
                .filter(|owned| owned.part == part.key)
            {
                let source = product
                    .features
                    .get(&owned.feature_id)
                    .ok_or(CanonicalError::InvalidFeatureMap)?;
                // Do not bless a pre-existing ownership conflict while cloning.
                if digest_feature(source) != owned.canonical_fingerprint {
                    return Err(CanonicalError::AssemblyRecipe(
                        AssemblyRecipeError::OwnedFeatureConflict(owned.key.clone()),
                    ));
                }
                owned.feature_id = *mapping
                    .get(&owned.feature_id)
                    .ok_or(CanonicalError::InvalidFeatureMap)?;
                owned.canonical_fingerprint = digest_feature(&product.features[&owned.feature_id]);
            }
        }
    }
    Ok(())
}

pub(super) fn five_feature_solid_tool_path(
    product: &ProductModel,
    target: &FeatureKind,
    tool: &FeatureKind,
) -> bool {
    match (target, tool) {
        (FeatureKind::ImportedExactBody(_), FeatureKind::ImportedExactBody(_)) => true,
        (FeatureKind::ImportedExactBody(_), pad) | (pad, FeatureKind::ImportedExactBody(_)) => {
            is_profile_feature_new_body(product, pad)
        }
        _ => false,
    }
}

/// A new-body pad of a standalone profile feature, with no face references:
/// the pad and its profile are a self-contained pair of features.
pub(super) fn is_profile_feature_new_body(product: &ProductModel, kind: &FeatureKind) -> bool {
    let FeatureKind::Pad(PadSpec {
        profile: PadProfile::Feature(profile),
        operation: PadOperation::NewBody,
        extent,
        ..
    }) = kind
    else {
        return false;
    };
    extent.references().is_empty()
        && product
            .features
            .get(profile)
            .is_some_and(|profile| matches!(profile.kind, FeatureKind::Profile { .. }))
}

pub(super) fn exact_solid_tool_dependency_closure_for_snapshot(
    snapshot: &Snapshot,
    feature_id: FeatureId,
) -> Result<Vec<FeatureId>, CanonicalError> {
    let product = &snapshot.product;
    let source = product
        .features
        .get(&feature_id)
        .ok_or(CanonicalError::FeatureNotFound(feature_id))?;
    ExactBRepGraph::from_snapshot(snapshot, source.definition_id, feature_id)
        .map_err(|error| CanonicalError::InvalidSolidToolPlan.because(error))?;
    let graph = snapshot.feature_dependency_graph()?;
    let mut closure = BTreeSet::from([feature_id]);
    let mut pending = vec![feature_id];
    while let Some(current) = pending.pop() {
        let dependencies = graph
            .dependencies(current)
            .ok_or(CanonicalError::InvalidSolidToolPlan)?;
        for dependency in dependencies {
            let dependency_feature = product
                .features
                .get(dependency)
                .ok_or(CanonicalError::FeatureNotFound(*dependency))?;
            if dependency_feature.definition_id != source.definition_id {
                return Err(CanonicalError::InvalidSolidToolPlan);
            }
            if closure.insert(*dependency) {
                pending.push(*dependency);
            }
        }
    }
    let ordered = graph
        .topological_order()
        .iter()
        .copied()
        .filter(|id| closure.contains(id))
        .collect::<Vec<_>>();
    if ordered.len() != closure.len() {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    let identity_mapping = ordered
        .iter()
        .copied()
        .map(|id| (id, id))
        .collect::<BTreeMap<_, _>>();
    for id in &ordered {
        let feature = product
            .features
            .get(id)
            .ok_or(CanonicalError::FeatureNotFound(*id))?;
        remap_exact_solid_tool_feature_kind(&feature.kind, &identity_mapping)?;
    }
    Ok(ordered)
}

pub(super) fn exact_solid_tool_dependency_closure(
    product: &ProductModel,
    feature_id: FeatureId,
) -> Result<Vec<FeatureId>, CanonicalError> {
    exact_solid_tool_dependency_closure_for_snapshot(
        &Snapshot {
            revision_id: 0,
            product: Arc::new(product.clone()),
        },
        feature_id,
    )
}

pub(super) fn solid_tool_result_feature_count(
    product: &ProductModel,
    target_feature_id: FeatureId,
    tool_feature_id: FeatureId,
) -> Result<usize, CanonicalError> {
    let target = product
        .features
        .get(&target_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(target_feature_id))?;
    let tool = product
        .features
        .get(&tool_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(tool_feature_id))?;
    if five_feature_solid_tool_path(product, &target.kind, &tool.kind) {
        return Ok(5);
    }
    let target_count = exact_solid_tool_dependency_closure(product, target_feature_id)?.len();
    let tool_count = exact_solid_tool_dependency_closure(product, tool_feature_id)?.len();
    let count = target_count
        .checked_add(tool_count)
        .and_then(|count| count.checked_add(3))
        .ok_or(CanonicalError::InvalidSolidToolPlan)?;
    if count > MAX_SOLID_TOOL_RESULT_FEATURES {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    Ok(count)
}

pub(super) fn remap_exact_solid_tool_feature_kind(
    kind: &FeatureKind,
    mapping: &BTreeMap<FeatureId, FeatureId>,
) -> Result<FeatureKind, CanonicalError> {
    let mapped = |id: &FeatureId| {
        mapping
            .get(id)
            .copied()
            .ok_or(CanonicalError::InvalidSolidToolPlan)
    };
    match kind {
        FeatureKind::Workplane(spec) => {
            let mut cloned = spec.clone();
            match &mut cloned.support {
                WorkplaneSupport::Free | WorkplaneSupport::Principal(_) => {}
                WorkplaneSupport::Offset { base, .. }
                | WorkplaneSupport::ConstructionPlane { feature: base } => *base = mapped(base)?,
                WorkplaneSupport::PlanarFace { .. } => {
                    return Err(CanonicalError::InvalidSolidToolPlan);
                }
            }
            Ok(FeatureKind::Workplane(cloned))
        }
        FeatureKind::Sketch(spec) => {
            let mut cloned = spec.clone();
            cloned.workplane = mapped(&spec.workplane)?;
            Ok(FeatureKind::Sketch(cloned))
        }
        FeatureKind::Profile { segments, closed } => Ok(FeatureKind::Profile {
            segments: segments.clone(),
            closed: *closed,
        }),
        FeatureKind::SpatialPath { segments } => Ok(FeatureKind::SpatialPath {
            segments: segments.clone(),
        }),
        FeatureKind::ConstructionPoint { position_mm } => Ok(FeatureKind::ConstructionPoint {
            position_mm: *position_mm,
        }),
        FeatureKind::ConstructionAxis {
            origin_mm,
            direction,
        } => Ok(FeatureKind::ConstructionAxis {
            origin_mm: *origin_mm,
            direction: *direction,
        }),
        FeatureKind::ConstructionPlane {
            origin_mm,
            normal,
            x_direction,
        } => Ok(FeatureKind::ConstructionPlane {
            origin_mm: *origin_mm,
            normal: *normal,
            x_direction: *x_direction,
        }),
        // Face references would still point into the source definition.
        FeatureKind::Pad(spec) if spec.references().is_empty() => {
            Ok(FeatureKind::Pad(spec.with_features(|id| mapped(&id))?))
        }
        FeatureKind::Revolve {
            profile,
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
        } => Ok(FeatureKind::Revolve {
            profile: mapped(profile)?,
            axis_start_mm: *axis_start_mm,
            axis_end_mm: *axis_end_mm,
            angle_degrees: *angle_degrees,
        }),
        FeatureKind::Boolean {
            operation,
            target,
            tool,
        } => Ok(FeatureKind::Boolean {
            operation: *operation,
            target: mapped(target)?,
            tool: mapped(tool)?,
        }),
        FeatureKind::Sweep { profile, path } => Ok(FeatureKind::Sweep {
            profile: mapped(profile)?,
            path: mapped(path)?,
        }),
        FeatureKind::Loft {
            sections,
            guide,
            continuity,
        } => Ok(FeatureKind::Loft {
            sections: sections
                .iter()
                .map(|section| {
                    Ok(LoftSection {
                        profile: mapped(&section.profile)?,
                        elevation_mm: section.elevation_mm,
                    })
                })
                .collect::<Result<Vec<_>, CanonicalError>>()?,
            guide: guide.map(|guide| mapped(&guide)).transpose()?,
            continuity: *continuity,
        }),
        FeatureKind::SheetMetal(spec) => Ok(FeatureKind::SheetMetal(spec.clone())),
        FeatureKind::ImportedExactBody(spec) => Ok(FeatureKind::ImportedExactBody(spec.clone())),
        FeatureKind::RigidTransform { target, transform } => Ok(FeatureKind::RigidTransform {
            target: mapped(target)?,
            transform: *transform,
        }),
        _ => Err(CanonicalError::InvalidSolidToolPlan),
    }
}

pub(super) fn clone_solid_tool_closure(
    product: &ProductModel,
    source_ids: &[FeatureId],
    output_ids: &[FeatureId],
    result_definition_id: DefinitionId,
) -> Result<(Vec<Feature>, BTreeMap<FeatureId, FeatureId>), CanonicalError> {
    if source_ids.len() != output_ids.len() {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    let mapping = source_ids
        .iter()
        .copied()
        .zip(output_ids.iter().copied())
        .collect::<BTreeMap<_, _>>();
    let features = source_ids
        .iter()
        .zip(output_ids)
        .map(|(source_id, output_id)| {
            let source = product
                .features
                .get(source_id)
                .ok_or(CanonicalError::FeatureNotFound(*source_id))?;
            Ok(Feature {
                id: *output_id,
                definition_id: result_definition_id,
                name: source.name.clone(),
                kind: remap_exact_solid_tool_feature_kind(&source.kind, &mapping)?,
            })
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;
    Ok((features, mapping))
}

pub(super) fn apply_graph_exact_solid_tool(
    product: &mut ProductModel,
    plan: &SolidToolPlan,
    target_occurrence: Occurrence,
    tool_occurrence: Occurrence,
) -> Result<(), CanonicalError> {
    let target_source_ids = exact_solid_tool_dependency_closure(product, plan.target_feature_id)?;
    let tool_source_ids = exact_solid_tool_dependency_closure(product, plan.tool_feature_id)?;
    let expected_count = target_source_ids
        .len()
        .checked_add(tool_source_ids.len())
        .and_then(|count| count.checked_add(3))
        .ok_or(CanonicalError::InvalidSolidToolPlan)?;
    if expected_count > MAX_SOLID_TOOL_RESULT_FEATURES
        || plan.result_feature_ids.len() != expected_count
    {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    let target_end = target_source_ids.len();
    let target_transform_id = plan.result_feature_ids[target_end];
    let tool_start = target_end + 1;
    let tool_end = tool_start + tool_source_ids.len();
    let tool_transform_id = plan.result_feature_ids[tool_end];
    let result_id = plan.result_feature_ids[tool_end + 1];
    let (mut features, target_mapping) = clone_solid_tool_closure(
        product,
        &target_source_ids,
        &plan.result_feature_ids[..target_end],
        plan.result_definition_id,
    )?;
    let (tool_features, tool_mapping) = clone_solid_tool_closure(
        product,
        &tool_source_ids,
        &plan.result_feature_ids[tool_start..tool_end],
        plan.result_definition_id,
    )?;
    features.extend(tool_features);

    let snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    let bounded_overlap_required = matches!(
        plan.operation,
        BooleanOperation::Intersect | BooleanOperation::Split
    ) && ExactBRepGraph::from_snapshot(
        &snapshot,
        target_occurrence.definition_id,
        plan.target_feature_id,
    )
    .and_then(|graph| graph.producer_bounds_mm())
    .map_err(|error| CanonicalError::InvalidSolidToolPlan.because(error))?
    .is_some()
        && ExactBRepGraph::from_snapshot(
            &snapshot,
            tool_occurrence.definition_id,
            plan.tool_feature_id,
        )
        .and_then(|graph| graph.producer_bounds_mm())
        .map_err(|error| CanonicalError::InvalidSolidToolPlan.because(error))?
        .is_some();
    let target_inverse = snapshot
        .world_transform_for_occurrence(plan.target_occurrence_id)
        .and_then(Transform::rigid_inverse)
        .ok_or(CanonicalError::UnsupportedSolidToolTransform)?;
    let tool_transform = snapshot
        .world_transform_for_occurrence(plan.tool_occurrence_id)
        .ok_or(CanonicalError::UnsupportedSolidToolTransform)?;
    let tool_relative_transform = target_inverse.compose(tool_transform);
    if tool_relative_transform.rigid_inverse().is_none() {
        return Err(CanonicalError::UnsupportedSolidToolTransform);
    }
    let target_body = *target_mapping
        .get(&plan.target_feature_id)
        .ok_or(CanonicalError::InvalidSolidToolPlan)?;
    let tool_body = *tool_mapping
        .get(&plan.tool_feature_id)
        .ok_or(CanonicalError::InvalidSolidToolPlan)?;
    features.extend([
        Feature {
            id: target_transform_id,
            definition_id: plan.result_definition_id,
            name: "Target body".to_owned(),
            kind: FeatureKind::RigidTransform {
                target: target_body,
                transform: Transform::identity(),
            },
        },
        Feature {
            id: tool_transform_id,
            definition_id: plan.result_definition_id,
            name: "Transformed tool".to_owned(),
            kind: FeatureKind::RigidTransform {
                target: tool_body,
                transform: tool_relative_transform,
            },
        },
        Feature {
            id: result_id,
            definition_id: plan.result_definition_id,
            name: plan.result_feature_name.clone(),
            kind: FeatureKind::Boolean {
                operation: plan.operation,
                target: target_transform_id,
                tool: tool_transform_id,
            },
        },
    ]);

    product.definitions.insert(
        plan.result_definition_id,
        Arc::new(Definition {
            id: plan.result_definition_id,
            name: plan.result_definition_name.clone(),
            feature_ids: plan.result_feature_ids.clone(),
            bodies: BTreeMap::from([(DEFAULT_BODY_ID, default_body())]),
            active_body_id: DEFAULT_BODY_ID,
            feature_body_ownership: BTreeMap::new(),
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }),
    );
    for feature in features {
        validate_feature_kind(&feature.kind, product.tolerance.linear_mm())?;
        product.features.insert(feature.id, Arc::new(feature));
    }
    let mut result_definition = product.definitions[&plan.result_definition_id]
        .as_ref()
        .clone();
    for feature_id in result_definition.feature_ids.clone() {
        let feature = &product.features[&feature_id];
        let ownership =
            inferred_feature_body_ownership(product, &result_definition, &feature.kind)?;
        result_definition
            .feature_body_ownership
            .insert(feature_id, ownership);
    }
    product
        .definitions
        .insert(plan.result_definition_id, Arc::new(result_definition));

    let cloned_bindings = target_mapping
        .iter()
        .chain(tool_mapping.iter())
        .flat_map(|(source_id, output_id)| {
            product
                .feature_parameter_bindings
                .values()
                .filter(move |binding| binding.target.feature_id == *source_id)
                .map(move |binding| {
                    let target = FeatureParameterTarget {
                        feature_id: *output_id,
                        path: binding.target.path.clone(),
                        value_type: binding.target.value_type,
                    };
                    (
                        target.clone(),
                        Arc::new(FeatureParameterBinding {
                            target,
                            derived_from: binding.derived_from.clone(),
                        }),
                    )
                })
        })
        .collect::<Vec<_>>();
    product.feature_parameter_bindings.extend(cloned_bindings);
    product.occurrences.insert(
        plan.target_occurrence_id,
        Arc::new(Occurrence {
            definition_id: plan.result_definition_id,
            ..target_occurrence
        }),
    );
    if !plan.keep_tool {
        product.occurrences.remove(&plan.tool_occurrence_id);
    } else {
        product
            .occurrences
            .insert(plan.tool_occurrence_id, Arc::new(tool_occurrence));
    }
    let result_snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    let result_graph =
        ExactBRepGraph::from_snapshot(&result_snapshot, plan.result_definition_id, result_id)
            .map_err(|error| CanonicalError::InvalidSolidToolPlan.because(error))?;
    if bounded_overlap_required
        && result_graph
            .producer_bounds_mm()
            .map_err(|error| CanonicalError::InvalidSolidToolPlan.because(error))?
            .is_none()
    {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    Ok(())
}

pub(super) fn apply_solid_tool(
    product: &mut ProductModel,
    plan: &SolidToolPlan,
) -> Result<(), CanonicalError> {
    ensure_name(&plan.result_definition_name)?;
    ensure_name(&plan.result_feature_name)?;
    ensure_product_id(plan.result_definition_id.0)?;
    if plan.target_occurrence_id == plan.tool_occurrence_id
        || (plan.operation == BooleanOperation::Split && !plan.keep_tool)
        || product.definitions.contains_key(&plan.result_definition_id)
    {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    if plan.result_feature_ids.len() > MAX_SOLID_TOOL_RESULT_FEATURES {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    let mut output_ids = BTreeSet::new();
    for id in &plan.result_feature_ids {
        ensure_product_id(id.0)?;
        if !output_ids.insert(*id) || product.features.contains_key(id) {
            return Err(CanonicalError::InvalidSolidToolPlan);
        }
    }
    if !plan.keep_tool
        && product
            .collections
            .values()
            .any(|collection| collection.occurrence_ids.contains(&plan.tool_occurrence_id))
    {
        return Err(CanonicalError::OccurrenceInCollection(
            plan.tool_occurrence_id,
        ));
    }

    let target_occurrence = product
        .occurrences
        .get(&plan.target_occurrence_id)
        .ok_or(CanonicalError::OccurrenceNotFound(
            plan.target_occurrence_id,
        ))?
        .as_ref()
        .clone();
    let tool_occurrence = product
        .occurrences
        .get(&plan.tool_occurrence_id)
        .ok_or(CanonicalError::OccurrenceNotFound(plan.tool_occurrence_id))?
        .as_ref()
        .clone();
    let target_feature = product
        .features
        .get(&plan.target_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(plan.target_feature_id))?;
    let tool_feature = product
        .features
        .get(&plan.tool_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(plan.tool_feature_id))?;
    if target_feature.definition_id != target_occurrence.definition_id
        || tool_feature.definition_id != tool_occurrence.definition_id
    {
        return Err(CanonicalError::OccurrenceDefinitionMismatch);
    }
    if plan.result_feature_ids.len()
        != solid_tool_result_feature_count(product, plan.target_feature_id, plan.tool_feature_id)?
    {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    if let (
        FeatureKind::ImportedExactBody(target_spec),
        FeatureKind::ImportedExactBody(tool_spec),
    ) = (&target_feature.kind, &tool_feature.kind)
    {
        return apply_imported_exact_solid_tool(
            product,
            plan,
            target_occurrence,
            tool_occurrence,
            target_feature.name.clone(),
            target_spec.clone(),
            tool_feature.name.clone(),
            tool_spec.clone(),
        );
    }
    if five_feature_solid_tool_path(product, &target_feature.kind, &tool_feature.kind) {
        return apply_mixed_exact_solid_tool(
            product,
            plan,
            target_occurrence,
            tool_occurrence,
            target_feature.name.clone(),
            target_feature.kind.clone(),
            tool_feature.name.clone(),
            tool_feature.kind.clone(),
        );
    }
    apply_graph_exact_solid_tool(product, plan, target_occurrence, tool_occurrence)
}

// Keep the validated plan and owned source feature parts explicit at this mutation boundary.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_mixed_exact_solid_tool(
    product: &mut ProductModel,
    plan: &SolidToolPlan,
    target_occurrence: Occurrence,
    tool_occurrence: Occurrence,
    target_name: String,
    target_kind: FeatureKind,
    tool_name: String,
    tool_kind: FeatureKind,
) -> Result<(), CanonicalError> {
    let snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    let target_inverse = snapshot
        .world_transform_for_occurrence(plan.target_occurrence_id)
        .and_then(Transform::rigid_inverse)
        .ok_or(CanonicalError::UnsupportedSolidToolTransform)?;
    let tool_transform = snapshot
        .world_transform_for_occurrence(plan.tool_occurrence_id)
        .ok_or(CanonicalError::UnsupportedSolidToolTransform)?;
    let tool_relative_transform = target_inverse.compose(tool_transform);
    if tool_relative_transform.rigid_inverse().is_none() {
        return Err(CanonicalError::UnsupportedSolidToolTransform);
    }

    let [first, second, third, transformed_tool, result] = plan.result_feature_ids.as_slice()
    else {
        return Err(CanonicalError::InvalidSolidToolPlan);
    };
    let (first, second, third, transformed_tool, result) =
        (*first, *second, *third, *transformed_tool, *result);
    let (mut features, target_body, tool_body, binding_mappings) = match (target_kind, tool_kind) {
        (FeatureKind::ImportedExactBody(target_spec), FeatureKind::Pad(tool_pad)) => {
            let PadProfile::Feature(profile) = tool_pad.profile else {
                return Err(CanonicalError::InvalidSolidToolPlan);
            };
            let source_profile = product
                .features
                .get(&profile)
                .ok_or(CanonicalError::FeatureNotFound(profile))?;
            let profile_kind = match source_profile.kind() {
                FeatureKind::Profile { .. } => source_profile.kind().clone(),
                _ => return Err(CanonicalError::InvalidSolidToolPlan),
            };
            (
                vec![
                    Feature {
                        id: first,
                        definition_id: plan.result_definition_id,
                        name: target_name,
                        kind: FeatureKind::ImportedExactBody(target_spec),
                    },
                    Feature {
                        id: second,
                        definition_id: plan.result_definition_id,
                        name: source_profile.name().to_owned(),
                        kind: profile_kind,
                    },
                    Feature {
                        id: third,
                        definition_id: plan.result_definition_id,
                        name: tool_name,
                        kind: FeatureKind::Pad(PadSpec {
                            profile: PadProfile::Feature(second),
                            ..tool_pad
                        }),
                    },
                ],
                first,
                third,
                vec![(profile, second), (plan.tool_feature_id, third)],
            )
        }
        (FeatureKind::Pad(target_pad), FeatureKind::ImportedExactBody(tool_spec)) => {
            let PadProfile::Feature(profile) = target_pad.profile else {
                return Err(CanonicalError::InvalidSolidToolPlan);
            };
            let source_profile = product
                .features
                .get(&profile)
                .ok_or(CanonicalError::FeatureNotFound(profile))?;
            let profile_kind = match source_profile.kind() {
                FeatureKind::Profile { .. } => source_profile.kind().clone(),
                _ => return Err(CanonicalError::InvalidSolidToolPlan),
            };
            (
                vec![
                    Feature {
                        id: first,
                        definition_id: plan.result_definition_id,
                        name: source_profile.name().to_owned(),
                        kind: profile_kind,
                    },
                    Feature {
                        id: second,
                        definition_id: plan.result_definition_id,
                        name: target_name,
                        kind: FeatureKind::Pad(PadSpec {
                            profile: PadProfile::Feature(first),
                            ..target_pad
                        }),
                    },
                    Feature {
                        id: third,
                        definition_id: plan.result_definition_id,
                        name: tool_name,
                        kind: FeatureKind::ImportedExactBody(tool_spec),
                    },
                ],
                second,
                third,
                vec![(profile, first), (plan.target_feature_id, second)],
            )
        }
        _ => return Err(CanonicalError::InvalidSolidToolPlan),
    };
    features.extend([
        Feature {
            id: transformed_tool,
            definition_id: plan.result_definition_id,
            name: "Transformed tool".to_owned(),
            kind: FeatureKind::RigidTransform {
                target: tool_body,
                transform: tool_relative_transform,
            },
        },
        Feature {
            id: result,
            definition_id: plan.result_definition_id,
            name: plan.result_feature_name.clone(),
            kind: FeatureKind::Boolean {
                operation: plan.operation,
                target: target_body,
                tool: transformed_tool,
            },
        },
    ]);

    product.definitions.insert(
        plan.result_definition_id,
        Arc::new(Definition {
            id: plan.result_definition_id,
            name: plan.result_definition_name.clone(),
            feature_ids: plan.result_feature_ids.to_vec(),
            bodies: BTreeMap::from([(DEFAULT_BODY_ID, default_body())]),
            active_body_id: DEFAULT_BODY_ID,
            feature_body_ownership: BTreeMap::new(),
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }),
    );
    for feature in features {
        validate_feature_kind(&feature.kind, product.tolerance.linear_mm())?;
        product.features.insert(feature.id, Arc::new(feature));
    }
    let mut result_definition = product.definitions[&plan.result_definition_id]
        .as_ref()
        .clone();
    for feature_id in result_definition.feature_ids.clone() {
        let feature = &product.features[&feature_id];
        let ownership =
            inferred_feature_body_ownership(product, &result_definition, &feature.kind)?;
        result_definition
            .feature_body_ownership
            .insert(feature_id, ownership);
    }
    product
        .definitions
        .insert(plan.result_definition_id, Arc::new(result_definition));
    let cloned_bindings = binding_mappings
        .into_iter()
        .flat_map(|(source_id, output_id)| {
            product
                .feature_parameter_bindings
                .values()
                .filter(move |binding| binding.target.feature_id == source_id)
                .map(move |binding| {
                    let target = FeatureParameterTarget {
                        feature_id: output_id,
                        path: binding.target.path.clone(),
                        value_type: binding.target.value_type,
                    };
                    (
                        target.clone(),
                        Arc::new(FeatureParameterBinding {
                            target,
                            derived_from: binding.derived_from.clone(),
                        }),
                    )
                })
        })
        .collect::<Vec<_>>();
    product.feature_parameter_bindings.extend(cloned_bindings);
    product.occurrences.insert(
        plan.target_occurrence_id,
        Arc::new(Occurrence {
            definition_id: plan.result_definition_id,
            ..target_occurrence
        }),
    );
    if !plan.keep_tool {
        product.occurrences.remove(&plan.tool_occurrence_id);
    } else {
        product
            .occurrences
            .insert(plan.tool_occurrence_id, Arc::new(tool_occurrence));
    }
    Ok(())
}

// Keep the validated plan and owned imported-body parts explicit at this mutation boundary.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_imported_exact_solid_tool(
    product: &mut ProductModel,
    plan: &SolidToolPlan,
    target_occurrence: Occurrence,
    tool_occurrence: Occurrence,
    target_name: String,
    target_spec: ImportedExactBodySpec,
    tool_name: String,
    tool_spec: ImportedExactBodySpec,
) -> Result<(), CanonicalError> {
    let snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    let target_transform = snapshot
        .world_transform_for_occurrence(plan.target_occurrence_id)
        .and_then(Transform::rigid_inverse)
        .ok_or(CanonicalError::UnsupportedSolidToolTransform)?;
    let tool_transform = snapshot
        .world_transform_for_occurrence(plan.tool_occurrence_id)
        .ok_or(CanonicalError::UnsupportedSolidToolTransform)?;
    let tool_relative_transform = target_transform.compose(tool_transform);
    if tool_relative_transform.rigid_inverse().is_none() {
        return Err(CanonicalError::UnsupportedSolidToolTransform);
    }
    let [target_source, target_body, tool_source, tool_body, result] =
        plan.result_feature_ids.as_slice()
    else {
        return Err(CanonicalError::InvalidSolidToolPlan);
    };
    let (target_source, target_body, tool_source, tool_body, result) = (
        *target_source,
        *target_body,
        *tool_source,
        *tool_body,
        *result,
    );
    let features = [
        Feature {
            id: target_source,
            definition_id: plan.result_definition_id,
            name: target_name.clone(),
            kind: FeatureKind::ImportedExactBody(target_spec),
        },
        Feature {
            id: target_body,
            definition_id: plan.result_definition_id,
            name: target_name,
            kind: FeatureKind::RigidTransform {
                target: target_source,
                transform: Transform::identity(),
            },
        },
        Feature {
            id: tool_source,
            definition_id: plan.result_definition_id,
            name: tool_name.clone(),
            kind: FeatureKind::ImportedExactBody(tool_spec),
        },
        Feature {
            id: tool_body,
            definition_id: plan.result_definition_id,
            name: tool_name,
            kind: FeatureKind::RigidTransform {
                target: tool_source,
                transform: tool_relative_transform,
            },
        },
        Feature {
            id: result,
            definition_id: plan.result_definition_id,
            name: plan.result_feature_name.clone(),
            kind: FeatureKind::Boolean {
                operation: plan.operation,
                target: target_body,
                tool: tool_body,
            },
        },
    ];
    product.definitions.insert(
        plan.result_definition_id,
        Arc::new(Definition {
            id: plan.result_definition_id,
            name: plan.result_definition_name.clone(),
            feature_ids: plan.result_feature_ids.to_vec(),
            bodies: BTreeMap::from([(DEFAULT_BODY_ID, default_body())]),
            active_body_id: DEFAULT_BODY_ID,
            feature_body_ownership: BTreeMap::new(),
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }),
    );
    for feature in features {
        validate_feature_kind(&feature.kind, product.tolerance.linear_mm())?;
        product.features.insert(feature.id, Arc::new(feature));
    }
    let mut result_definition = product.definitions[&plan.result_definition_id]
        .as_ref()
        .clone();
    for feature_id in result_definition.feature_ids.clone() {
        let feature = &product.features[&feature_id];
        let ownership =
            inferred_feature_body_ownership(product, &result_definition, &feature.kind)?;
        result_definition
            .feature_body_ownership
            .insert(feature_id, ownership);
    }
    product
        .definitions
        .insert(plan.result_definition_id, Arc::new(result_definition));
    product.occurrences.insert(
        plan.target_occurrence_id,
        Arc::new(Occurrence {
            definition_id: plan.result_definition_id,
            ..target_occurrence
        }),
    );
    if !plan.keep_tool {
        product.occurrences.remove(&plan.tool_occurrence_id);
    } else {
        product
            .occurrences
            .insert(plan.tool_occurrence_id, Arc::new(tool_occurrence));
    }
    Ok(())
}
