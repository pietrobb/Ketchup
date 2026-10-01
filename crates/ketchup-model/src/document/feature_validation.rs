use super::*;

pub(super) const MAX_STABLE_SUBSHAPE_ROLE_BYTES: usize = 128;

pub(super) fn validate_stable_subshape_role(role: &str) -> Result<(), CanonicalError> {
    if role.is_empty()
        || role.len() > MAX_STABLE_SUBSHAPE_ROLE_BYTES
        || !role.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-' | b'=' | b'(' | b')' | b',' | b':')
        })
    {
        return Err(CanonicalError::InvalidStableSubshapeRole);
    }
    Ok(())
}

pub(super) fn roles_are_strictly_sorted<T: Ord>(roles: &[T]) -> bool {
    !roles.is_empty() && roles.windows(2).all(|pair| pair[0] < pair[1])
}

pub(super) fn validate_topological_feature_references(
    references: &[TopologicalElementRef],
    expected_kind: TopologicalElementKind,
) -> Result<(), CanonicalError> {
    if references.len() > 64
        || !roles_are_strictly_sorted(references)
        || references
            .iter()
            .any(|reference| reference.kind != expected_kind || !reference.has_valid_lineage())
    {
        return Err(CanonicalError::InvalidTopologicalFeatureReference);
    }
    Ok(())
}

pub(super) fn validate_topological_feature_context(
    document_id: DocumentId,
    definition_id: DefinitionId,
    kind: &FeatureKind,
) -> Result<(), CanonicalError> {
    let Some((target, references)) = kind.topological_picks() else {
        return Ok(());
    };
    let chamfer_edge_sides = match kind {
        FeatureKind::EdgeFinish {
            chamfer_edge_sides, ..
        } => Some(chamfer_edge_sides.as_slice()),
        _ => None,
    };
    let invalid_context = |reference: &TopologicalElementRef| {
        reference.document_id != document_id
            || reference.definition_id != definition_id
            || reference.producer_feature_id != target
    };
    if references
        .iter()
        .any(|reference| invalid_context(reference))
        || chamfer_edge_sides.is_some_and(|selections| {
            selections.iter().any(|selection| {
                invalid_context(&selection.edge) || invalid_context(&selection.side_face)
            })
        })
    {
        return Err(CanonicalError::InvalidTopologicalFeatureReference);
    }
    Ok(())
}

pub(super) fn validate_topological_target(
    product: &ProductModel,
    definition: &Definition,
    feature_id: FeatureId,
    target_id: FeatureId,
    references: &[&TopologicalElementRef],
) -> Result<(), CanonicalError> {
    let target = product
        .features
        .get(&target_id)
        .ok_or(CanonicalError::FeatureNotFound(target_id))?;
    let feature_position = definition
        .feature_ids
        .iter()
        .position(|candidate| *candidate == feature_id)
        .ok_or(CanonicalError::InvalidFeatureOwnership(feature_id))?;
    let target_position = definition
        .feature_ids
        .iter()
        .position(|candidate| *candidate == target_id);
    let sources_are_valid = references.iter().all(|reference| {
        product
            .features
            .get(&reference.source_feature_id)
            .is_some_and(|source| {
                source.definition_id == target.definition_id
                    && definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == source.id)
                        .is_some_and(|position| position <= target_position.unwrap_or(usize::MAX))
            })
    });
    if target_id == feature_id
        || target.definition_id != definition.id
        || !feature_kind_is_solid(&target.kind)
        || target_position.is_none_or(|position| position >= feature_position)
        || !sources_are_valid
    {
        return Err(CanonicalError::InvalidFeatureOwnership(feature_id));
    }
    Ok(())
}

pub(super) fn feature_kind_is_solid(kind: &FeatureKind) -> bool {
    matches!(
        kind,
        FeatureKind::Pad(_)
            | FeatureKind::Revolve { .. }
            | FeatureKind::Shell { .. }
            | FeatureKind::EdgeFinish { .. }
            | FeatureKind::FaceOffset { .. }
            | FeatureKind::SurfaceThicken { .. }
            | FeatureKind::Boolean { .. }
            | FeatureKind::Sweep { .. }
            | FeatureKind::WeldmentMember(_)
            | FeatureKind::WeldmentJoint(_)
            | FeatureKind::Loft { .. }
            | FeatureKind::SheetMetal(_)
            | FeatureKind::ImportedExactBody(_)
            | FeatureKind::RigidTransform { .. }
            | FeatureKind::MeshBody(_)
    )
}

/// The profile, sketch, target and support a pad names exist in its
/// definition and fit together.
pub(super) fn pad_inputs_are_valid(
    product: &ProductModel,
    definition_id: DefinitionId,
    spec: &PadSpec,
) -> Result<bool, CanonicalError> {
    let local = |id: FeatureId| {
        product
            .features
            .get(&id)
            .ok_or(CanonicalError::FeatureNotFound(id))
            .map(|feature| (feature.definition_id == definition_id).then_some(feature))
    };
    let (sketch_workplane, profile_valid) = match spec.profile {
        PadProfile::Feature(id) => {
            let Some(profile) = local(id)? else {
                return Ok(false);
            };
            match &profile.kind {
                FeatureKind::Profile { closed: true, .. } => (None, true),
                FeatureKind::Sketch(sketch) => (
                    Some(sketch.workplane),
                    sketch
                        .solved_regions()
                        .is_ok_and(|regions| regions.len() == 1),
                ),
                _ => (None, false),
            }
        }
        PadProfile::SketchRegion { sketch, region } => {
            let Some(sketch) = local(sketch)? else {
                return Ok(false);
            };
            let FeatureKind::Sketch(sketch) = &sketch.kind else {
                return Ok(false);
            };
            let region_exists = sketch
                .solved_regions()
                .map_err(CanonicalError::from)?
                .iter()
                .any(|solved| solved.id == region);
            (Some(sketch.workplane), region_exists)
        }
    };
    let workplane = match sketch_workplane {
        Some(id) => match local(id)?.map(|feature| &feature.kind) {
            Some(FeatureKind::Workplane(workplane)) => Some(workplane),
            _ => return Ok(false),
        },
        None => None,
    };
    if !profile_valid {
        return Ok(false);
    }
    let PadOperation::Cut { target, start } = &spec.operation else {
        return Ok(true);
    };
    let Some(target_feature) = local(*target)? else {
        return Ok(false);
    };
    if !feature_kind_is_solid(&target_feature.kind) {
        return Ok(false);
    }
    Ok(match start {
        CutStart::ProfilePlane => true,
        CutStart::Support(support) => {
            support.producer_feature_id == *target
                && matches!(
                    workplane,
                    Some(WorkplaneSpec {
                        support: WorkplaneSupport::PlanarFace { reference, health },
                        ..
                    }) if reference.lineage_digest == support.lineage_digest
                        && (*health != WorkplaneSupportHealth::Resolved
                            || reference.as_ref() == support.as_ref())
                )
        }
        // A blind cut must stay inside the target. Against a blind new body
        // that is checked here; any other target's extent is only known to
        // the exact graph compiler.
        CutStart::TargetFace => match (&spec.extent, &target_feature.kind) {
            (
                FeatureExtent::Blind(depth),
                FeatureKind::Pad(PadSpec {
                    extent: FeatureExtent::Blind(height),
                    operation: PadOperation::NewBody,
                    ..
                }),
            ) => depth.millimetres() < height.millimetres(),
            (extent, _) => matches!(extent, FeatureExtent::Blind(_)),
        },
    })
}

pub(super) fn primary_solid_dependency(kind: &FeatureKind) -> Option<FeatureId> {
    match kind {
        FeatureKind::Pad(spec) => spec.operation.target(),
        FeatureKind::Shell { target, .. }
        | FeatureKind::EdgeFinish { target, .. }
        | FeatureKind::FaceOffset { target, .. }
        | FeatureKind::SurfaceThicken { target, .. }
        | FeatureKind::Boolean { target, .. }
        | FeatureKind::RigidTransform { target, .. } => Some(*target),
        _ => None,
    }
}

pub(crate) fn migrate_legacy_body_contract(
    product: &mut ProductModel,
) -> Result<(), CanonicalError> {
    let definition_ids = product.definitions.keys().cloned().collect::<Vec<_>>();
    for definition_id in definition_ids {
        let mut definition = product.definitions[&definition_id].as_ref().clone();
        definition.bodies = BTreeMap::from([(DEFAULT_BODY_ID, default_body())]);
        definition.active_body_id = DEFAULT_BODY_ID;
        definition.feature_body_ownership.clear();
        for feature_id in definition.feature_ids.clone() {
            let feature = product
                .features
                .get(&feature_id)
                .ok_or(CanonicalError::FeatureNotFound(feature_id))?;
            let ownership = inferred_feature_body_ownership(product, &definition, &feature.kind)?;
            definition
                .feature_body_ownership
                .insert(feature_id, ownership);
        }
        product
            .definitions
            .insert(definition_id, Arc::new(definition));
    }
    Ok(())
}

pub(super) fn inferred_feature_body_ownership(
    _product: &ProductModel,
    definition: &Definition,
    kind: &FeatureKind,
) -> Result<FeatureBodyOwnership, CanonicalError> {
    let input_body_ids = kind
        .dependencies()
        .into_iter()
        .filter_map(|dependency| {
            definition
                .feature_body_ownership
                .get(&dependency)
                .and_then(FeatureBodyOwnership::output_body_id)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let output_body_id = if kind.produces_body() {
        primary_solid_dependency(kind)
            .and_then(|dependency| {
                definition
                    .feature_body_ownership
                    .get(&dependency)
                    .and_then(FeatureBodyOwnership::output_body_id)
            })
            .or(Some(definition.active_body_id))
    } else {
        None
    };
    FeatureBodyOwnership::new(input_body_ids, output_body_id)
}

pub(super) fn feature_references_are_resolved(
    product: &ProductModel,
    id: FeatureId,
    visited: &mut BTreeSet<FeatureId>,
) -> bool {
    if !visited.insert(id) {
        return true;
    }
    let Some(feature) = product.features.get(&id) else {
        return false;
    };
    if matches!(
        &feature.kind,
        FeatureKind::Workplane(WorkplaneSpec {
            support: WorkplaneSupport::PlanarFace { health, .. },
            ..
        }) if *health != WorkplaneSupportHealth::Resolved
    ) {
        return false;
    }
    feature
        .kind
        .authoritative_dependencies()
        .into_iter()
        .all(|dependency| feature_references_are_resolved(product, dependency, visited))
}

pub(super) fn validate_feature_body_ownership_change(
    product: &ProductModel,
    feature: &Feature,
    ownership: &FeatureBodyOwnership,
) -> Result<(), CanonicalError> {
    let definition = &product.definitions[&feature.definition_id];
    if ownership
        .input_body_ids
        .iter()
        .chain(ownership.output_body_id.iter())
        .any(|id| !definition.bodies.contains_key(id))
    {
        return Err(CanonicalError::BodyNotFound(
            definition.id,
            ownership
                .input_body_ids
                .iter()
                .chain(ownership.output_body_id.iter())
                .find(|id| !definition.bodies.contains_key(id))
                .cloned()
                .expect("a missing body was detected"),
        ));
    }
    if ownership
        .input_body_ids
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(CanonicalError::BodyInputsNotCanonical);
    }
    let inferred = inferred_feature_body_ownership(product, definition, &feature.kind)?;
    if ownership.input_body_ids != inferred.input_body_ids
        || feature.kind.produces_body() != ownership.output_body_id.is_some()
    {
        return Err(CanonicalError::InvalidBodyOwnership(feature.id));
    }
    if !feature_references_are_resolved(product, feature.id, &mut BTreeSet::new()) {
        return Err(CanonicalError::UnresolvedBodyOwnershipReference(feature.id));
    }
    let mut candidate = definition.as_ref().clone();
    candidate
        .feature_body_ownership
        .insert(feature.id, ownership.clone());
    validate_body_dependency_graph(&candidate)?;
    Ok(())
}

pub(super) fn ordered_body_feature_history(
    product: &ProductModel,
    definition_id: DefinitionId,
    body_id: BodyId,
    graph: &FeatureDependencyGraph,
) -> Result<Vec<FeatureId>, CanonicalError> {
    let definition = product
        .definitions
        .get(&definition_id)
        .ok_or(CanonicalError::DefinitionNotFound(definition_id))?;
    if !definition.bodies.contains_key(&body_id) {
        return Err(CanonicalError::BodyNotFound(definition_id, body_id));
    }
    let mut history = BTreeSet::new();
    let mut pending = graph
        .topological_order()
        .iter()
        .cloned()
        .filter(|id| {
            definition
                .feature_body_ownership
                .get(id)
                .and_then(FeatureBodyOwnership::output_body_id)
                == Some(body_id)
        })
        .collect::<Vec<_>>();
    while let Some(feature_id) = pending.pop() {
        if !history.insert(feature_id) {
            continue;
        }
        for dependency in graph
            .dependencies(feature_id)
            .ok_or(CanonicalError::FeatureNotFound(feature_id))?
        {
            let ownership = definition
                .feature_body_ownership
                .get(dependency)
                .ok_or(CanonicalError::FeatureNotFound(*dependency))?;
            if ownership
                .output_body_id()
                .is_none_or(|output| output == body_id)
            {
                pending.push(*dependency);
            }
        }
    }
    Ok(graph
        .topological_order()
        .iter()
        .cloned()
        .filter(|id| history.contains(id))
        .collect())
}

pub(super) fn validate_body_feature_suppression(
    product: &ProductModel,
    definition_id: DefinitionId,
    body_id: BodyId,
    suppressed_feature_ids: &[FeatureId],
    graph: &FeatureDependencyGraph,
) -> Result<(), CanonicalError> {
    let ordered_history = ordered_body_feature_history(product, definition_id, body_id, graph)?;
    if suppressed_feature_ids.is_empty() {
        return Ok(());
    }
    let Some(boundary) = ordered_history
        .iter()
        .position(|id| *id == suppressed_feature_ids[0])
    else {
        return Err(CanonicalError::InvalidFeatureSuppression(
            definition_id,
            body_id,
        ));
    };
    if ordered_history[boundary..] != *suppressed_feature_ids {
        return Err(CanonicalError::InvalidFeatureSuppression(
            definition_id,
            body_id,
        ));
    }
    let suppressed = suppressed_feature_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if suppressed.len() != suppressed_feature_ids.len()
        || suppressed.iter().any(|id| {
            graph.dependents(*id).is_some_and(|dependents| {
                dependents
                    .iter()
                    .any(|dependent| !suppressed.contains(dependent))
            })
        })
    {
        return Err(CanonicalError::InvalidFeatureSuppression(
            definition_id,
            body_id,
        ));
    }
    Ok(())
}

pub(super) fn validate_body_dependency_graph(
    definition: &Definition,
) -> Result<(), CanonicalError> {
    let mut edges = definition
        .bodies
        .keys()
        .cloned()
        .map(|id| (id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = definition
        .bodies
        .keys()
        .cloned()
        .map(|id| (id, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for ownership in definition.feature_body_ownership.values() {
        let Some(output) = ownership.output_body_id else {
            continue;
        };
        for input in &ownership.input_body_ids {
            if *input != output && edges.get_mut(input).is_some_and(|set| set.insert(output)) {
                *indegree
                    .get_mut(&output)
                    .expect("validated body output exists") += 1;
            }
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect::<BTreeSet<_>>();
    let mut visited = 0;
    while let Some(id) = ready.pop_first() {
        visited += 1;
        for dependent in &edges[&id] {
            let degree = indegree
                .get_mut(dependent)
                .expect("body dependency target exists");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(*dependent);
            }
        }
    }
    if visited != definition.bodies.len() {
        return Err(CanonicalError::BodyDependencyCycle(definition.id));
    }
    Ok(())
}

pub(super) fn sketch_workplane_frame(
    product: &ProductModel,
    sketch_feature_id: FeatureId,
) -> Result<WorkplaneFrame, CanonicalError> {
    let sketch = product
        .features
        .get(&sketch_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(sketch_feature_id))?;
    let FeatureKind::Sketch(spec) = &sketch.kind else {
        return Err(CanonicalError::FeatureIsNotProfile(sketch_feature_id));
    };
    let workplane_id = spec.workplane;
    let workplane = product
        .features
        .get(&workplane_id)
        .ok_or(CanonicalError::Sketch(
            SketchError::MissingWorkplaneSupport(workplane_id),
        ))?;
    let FeatureKind::Workplane(spec) = &workplane.kind else {
        return Err(CanonicalError::Sketch(
            SketchError::MissingWorkplaneSupport(workplane_id),
        ));
    };
    spec.validate_local()?;
    Ok(spec.frame)
}

pub(super) fn project_sketch_entity(
    product: &ProductModel,
    target_feature_id: FeatureId,
    source_feature_id: FeatureId,
    source_entity_id: ketchup_geometry::sketch::SketchEntityId,
    new_entity_id: ketchup_geometry::sketch::SketchEntityId,
) -> Result<SketchEntity, CanonicalError> {
    if target_feature_id == source_feature_id || source_entity_id.0 == 0 || new_entity_id.0 == 0 {
        return Err(CanonicalError::Sketch(SketchError::InvalidProjectionSource));
    }
    let target_feature = product
        .features
        .get(&target_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(target_feature_id))?;
    let source_feature = product
        .features
        .get(&source_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(source_feature_id))?;
    if target_feature.definition_id != source_feature.definition_id {
        return Err(CanonicalError::Sketch(SketchError::InvalidProjectionSource));
    }
    let FeatureKind::Sketch(source_spec) = &source_feature.kind else {
        return Err(CanonicalError::FeatureIsNotProfile(source_feature_id));
    };
    let source = source_spec
        .entities
        .iter()
        .find(|entity| entity.id() == source_entity_id)
        .ok_or(CanonicalError::Sketch(SketchError::EntityNotFound(
            source_entity_id,
        )))?;
    let source_frame = sketch_workplane_frame(product, source_feature_id)?;
    let target_frame = sketch_workplane_frame(product, target_feature_id)?;
    let dot3 = ketchup_geometry::linalg::dot::<[f64; 3]>;
    let project_point = |point: [f64; 2]| {
        let world = [
            source_frame.origin_mm[0]
                + source_frame.x_axis[0] * point[0]
                + source_frame.y_axis[0] * point[1],
            source_frame.origin_mm[1]
                + source_frame.x_axis[1] * point[0]
                + source_frame.y_axis[1] * point[1],
            source_frame.origin_mm[2]
                + source_frame.x_axis[2] * point[0]
                + source_frame.y_axis[2] * point[1],
        ];
        let delta = [
            world[0] - target_frame.origin_mm[0],
            world[1] - target_frame.origin_mm[1],
            world[2] - target_frame.origin_mm[2],
        ];
        [
            dot3(delta, target_frame.x_axis),
            dot3(delta, target_frame.y_axis),
        ]
        .map(|value| if value == 0.0 { 0.0 } else { value })
    };
    let normal_alignment = dot3(source_frame.normal, target_frame.normal);
    let projected = match source {
        SketchEntity::Line {
            start_mm, end_mm, ..
        } => SketchEntity::Line {
            id: new_entity_id,
            start_mm: project_point(*start_mm),
            end_mm: project_point(*end_mm),
        },
        SketchEntity::Arc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
            ..
        } if normal_alignment.abs() >= 1.0 - ROUNDING => SketchEntity::Arc {
            id: new_entity_id,
            start_mm: project_point(*start_mm),
            end_mm: project_point(*end_mm),
            center_mm: project_point(*center_mm),
            clockwise: if normal_alignment < 0.0 {
                !clockwise
            } else {
                *clockwise
            },
        },
        SketchEntity::Circle {
            center_mm,
            radius_mm,
            ..
        } if normal_alignment.abs() >= 1.0 - ROUNDING => SketchEntity::Circle {
            id: new_entity_id,
            center_mm: project_point(*center_mm),
            radius_mm: *radius_mm,
        },
        SketchEntity::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } => SketchEntity::CubicBezier {
            id: new_entity_id,
            start_mm: project_point(*start_mm),
            control_1_mm: project_point(*control_1_mm),
            control_2_mm: project_point(*control_2_mm),
            end_mm: project_point(*end_mm),
        },
        SketchEntity::Arc { .. } | SketchEntity::Circle { .. } => {
            return Err(CanonicalError::Sketch(SketchError::InvalidProjectionSource));
        }
    };
    Ok(projected)
}

pub(super) fn refresh_sketch_projections(product: &mut ProductModel) -> Result<(), CanonicalError> {
    let graph = FeatureDependencyGraph::from_product(product)?;
    for feature_id in graph.topological_order().iter().copied() {
        let Some(feature) = product.features.get(&feature_id).cloned() else {
            continue;
        };
        let FeatureKind::Sketch(spec) = &feature.kind else {
            continue;
        };
        let projections = spec
            .constraints
            .iter()
            .filter_map(|constraint| match constraint.kind {
                SketchConstraintKind::Projection {
                    entity,
                    source_feature,
                    source_entity,
                    ..
                } => Some((entity, source_feature, source_entity)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if projections.is_empty() {
            continue;
        }
        let mut updated = spec.clone();
        for (entity, source_feature, source_entity) in projections {
            let target =
                project_sketch_entity(product, feature_id, source_feature, source_entity, entity)?;
            updated.refresh_projection(entity, target)?;
        }
        product.features.insert(
            feature_id,
            Arc::new(Feature {
                kind: FeatureKind::Sketch(updated),
                ..feature.as_ref().clone()
            }),
        );
    }
    Ok(())
}

pub(super) fn validate_sketch_constraint_edit_dependents(
    product: &ProductModel,
    sketch_id: FeatureId,
    updated: &SketchSpec,
    constraint_id: SketchConstraintId,
) -> Result<(), CanonicalError> {
    let required_regions = product
        .features
        .values()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::Pad(PadSpec {
                profile: PadProfile::SketchRegion { sketch, region },
                ..
            }) if *sketch == sketch_id => Some(*region),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if required_regions.is_empty() {
        return Ok(());
    }
    let available_regions = updated
        .solved_regions()
        .map_err(|error| {
            CanonicalError::Sketch(SketchError::ConstraintEditInvalidatesProfile(
                constraint_id,
                Some(Box::new(error)),
            ))
        })?
        .into_iter()
        .map(|region| region.id)
        .collect::<BTreeSet<_>>();
    if required_regions.is_subset(&available_regions) {
        Ok(())
    } else {
        Err(CanonicalError::Sketch(
            SketchError::ConstraintEditInvalidatesProfile(constraint_id, None),
        ))
    }
}

pub(super) fn validate_sketch_projections(product: &ProductModel) -> Result<(), CanonicalError> {
    FeatureDependencyGraph::from_product(product)?;
    for (feature_id, feature) in &product.features {
        let FeatureKind::Sketch(spec) = &feature.kind else {
            continue;
        };
        for constraint in &spec.constraints {
            let SketchConstraintKind::Projection {
                entity,
                source_feature,
                source_entity,
                target,
            } = &constraint.kind
            else {
                continue;
            };
            let expected = project_sketch_entity(
                product,
                *feature_id,
                *source_feature,
                *source_entity,
                *entity,
            )?;
            let actual = spec
                .entities
                .iter()
                .find(|candidate| candidate.id() == *entity)
                .ok_or(CanonicalError::Sketch(SketchError::InvalidProjectionSource))?;
            if actual != &expected || target.as_ref() != &expected {
                return Err(CanonicalError::Sketch(SketchError::InvalidProjectionSource));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_feature_kind(
    kind: &FeatureKind,
    tolerance_mm: f64,
) -> Result<(), CanonicalError> {
    match kind {
        FeatureKind::Workplane(spec) => {
            let mut canonical = spec.clone();
            if let WorkplaneSupport::PlanarFace {
                reference,
                health: _,
            } = &canonical.support
            {
                canonical.support = WorkplaneSupport::PlanarFace {
                    reference: reference.clone(),
                    health: WorkplaneSupportHealth::Resolved,
                };
            }
            canonical.validate_local().map_err(CanonicalError::from)
        }
        FeatureKind::Sketch(spec) => spec.solve().map(|_| ()).map_err(CanonicalError::from),
        FeatureKind::Profile { segments, closed } => {
            // A closed chain of lines is a polygon, and a closed spline runs around the
            // polygon of its points: that polygon must be simple and enclose an area.
            if !is_valid_segment_profile(segments, *closed)
                || kind
                    .polygon_points()
                    .as_deref()
                    .or(kind.closed_spline_points())
                    .is_some_and(|points| !is_valid_profile(points))
            {
                return Err(CanonicalError::InvalidProfile);
            }
            Ok(())
        }
        FeatureKind::SpatialPath { segments } => {
            if !is_valid_spatial_sweep_path(segments, tolerance_mm) {
                return Err(CanonicalError::InvalidSweep);
            }
            Ok(())
        }
        FeatureKind::ConstructionPoint { position_mm } => {
            if position_mm
                .iter()
                .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE_MM)
            {
                return Err(CanonicalError::InvalidConstructionGeometry);
            }
            Ok(())
        }
        FeatureKind::ConstructionAxis {
            origin_mm,
            direction,
        } => {
            let direction_length_squared = direction.iter().map(|value| value * value).sum::<f64>();
            if origin_mm
                .iter()
                .chain(direction)
                .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE_MM)
                || !direction_length_squared.is_finite()
                || direction_length_squared <= f64::EPSILON
            {
                return Err(CanonicalError::InvalidConstructionGeometry);
            }
            Ok(())
        }
        FeatureKind::ConstructionPlane {
            origin_mm,
            normal,
            x_direction,
        } => {
            let normal_length_squared = normal.iter().map(|value| value * value).sum::<f64>();
            let x_length_squared = x_direction.iter().map(|value| value * value).sum::<f64>();
            let dot = normal
                .iter()
                .zip(x_direction)
                .map(|(normal, x)| normal * x)
                .sum::<f64>();
            if origin_mm
                .iter()
                .chain(normal)
                .chain(x_direction)
                .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE_MM)
                || !normal_length_squared.is_finite()
                || normal_length_squared <= f64::EPSILON
                || !x_length_squared.is_finite()
                || x_length_squared <= f64::EPSILON
                || !dot.is_finite()
                || dot.abs() > ROUNDING * (normal_length_squared * x_length_squared).sqrt()
            {
                return Err(CanonicalError::InvalidConstructionGeometry);
            }
            Ok(())
        }
        FeatureKind::Pad(spec) => {
            spec.direction.validate().map_err(CanonicalError::from)?;
            spec.extent.validate().map_err(CanonicalError::from)?;
            // A cut measured back from the target face reaches into the target.
            if let (
                PadOperation::Cut {
                    start: CutStart::TargetFace,
                    ..
                },
                FeatureExtent::Blind(depth),
            ) = (&spec.operation, &spec.extent)
                && depth.millimetres() <= 0.0
            {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            if let Some(support) = spec.operation.support()
                && (support.expected_type != "planar_face"
                    || support.expected_cardinality != 1
                    || !support.has_valid_lineage())
            {
                return Err(CanonicalError::Sketch(
                    SketchError::InvalidPlanarFaceSupport,
                ));
            }
            Ok(())
        }
        FeatureKind::Shell {
            removed_faces,
            thickness,
            direction,
            ..
        } => {
            Dimension::new(thickness.source_token(), thickness.millimetres()).map(|_| ())?;
            if thickness.millimetres() <= 0.0 {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            // No removed faces is a closed shell.
            if removed_faces.is_empty() {
                return Ok(());
            }
            if let Some(recorded) = FaceRef::all_topological(removed_faces) {
                return validate_topological_feature_references(
                    &recorded,
                    TopologicalElementKind::Face,
                );
            }
            // Named openings: an inward shell of program faces only.
            let named = FaceRef::all_named(removed_faces)
                .ok_or(CanonicalError::InvalidTopologicalFeatureReference)?;
            if *direction != ShellDirection::Inward
                || named.len() > 64
                || !named.iter().all(ProfileFaceReference::is_valid)
            {
                return Err(CanonicalError::InvalidTopologicalFeatureReference);
            }
            Ok(())
        }
        FeatureKind::EdgeFinish {
            edges,
            kind,
            amount,
            fillet_radius_stations,
            chamfer_mode,
            chamfer_edge_sides,
            ..
        } => {
            Dimension::new(amount.source_token(), amount.millimetres()).map(|_| ())?;
            if !(EXACT_MIN_LENGTH_MM..=MAX_EXACT_PLANAR_OFFSET_LENGTH_MM)
                .contains(&amount.millimetres())
            {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            match kind {
                EdgeFinishKind::Fillet
                    if *chamfer_mode != ChamferMode::Symmetric
                        || !chamfer_edge_sides.is_empty() =>
                {
                    return Err(CanonicalError::DimensionOutsideEnvelope);
                }
                EdgeFinishKind::Chamfer if !fillet_radius_stations.is_empty() => {
                    return Err(CanonicalError::DimensionOutsideEnvelope);
                }
                EdgeFinishKind::Chamfer => match chamfer_mode {
                    ChamferMode::Symmetric if !chamfer_edge_sides.is_empty() => {
                        return Err(CanonicalError::InvalidTopologicalFeatureReference);
                    }
                    ChamferMode::TwoDistance { second_distance } => {
                        Dimension::new(
                            second_distance.source_token(),
                            second_distance.millimetres(),
                        )
                        .map(|_| ())?;
                        if !(EXACT_MIN_LENGTH_MM..=MAX_EXACT_PLANAR_OFFSET_LENGTH_MM)
                            .contains(&second_distance.millimetres())
                        {
                            return Err(CanonicalError::DimensionOutsideEnvelope);
                        }
                    }
                    ChamferMode::DistanceAngle { angle_degrees }
                        if !angle_degrees.is_finite()
                            || *angle_degrees <= 0.1
                            || *angle_degrees >= 89.9 =>
                    {
                        return Err(CanonicalError::DimensionOutsideEnvelope);
                    }
                    ChamferMode::Symmetric | ChamferMode::DistanceAngle { .. } => {}
                },
                EdgeFinishKind::Fillet => {}
            }
            if !matches!(chamfer_mode, ChamferMode::Symmetric)
                && (chamfer_edge_sides.len() != edges.len()
                    || chamfer_edge_sides
                        .iter()
                        .zip(edges)
                        .any(|(selection, edge)| {
                            edge.topological() != Some(&selection.edge)
                                || selection.side_face.kind != TopologicalElementKind::Face
                                || !selection.side_face.has_valid_lineage()
                        }))
            {
                return Err(CanonicalError::InvalidTopologicalFeatureReference);
            }
            if !fillet_radius_stations.is_empty() {
                if fillet_radius_stations.len() > 32
                    || fillet_radius_stations
                        .last()
                        .is_none_or(|station| station.position != 1.0)
                    || fillet_radius_stations
                        .windows(2)
                        .any(|pair| pair[0].position >= pair[1].position)
                {
                    return Err(CanonicalError::DimensionOutsideEnvelope);
                }
                let mut previous = 0.0;
                for station in fillet_radius_stations {
                    Dimension::new(station.radius.source_token(), station.radius.millimetres())
                        .map(|_| ())?;
                    if !station.position.is_finite()
                        || station.position <= previous
                        || station.position > 1.0
                        || !(EXACT_MIN_LENGTH_MM..=MAX_EXACT_PLANAR_OFFSET_LENGTH_MM)
                            .contains(&station.radius.millimetres())
                    {
                        return Err(CanonicalError::DimensionOutsideEnvelope);
                    }
                    previous = station.position;
                }
            }
            if edges.is_empty() {
                return Err(CanonicalError::InvalidTopologicalFeatureReference);
            }
            if let Some(recorded) = EdgeRef::all_topological(edges) {
                return validate_topological_feature_references(
                    &recorded,
                    TopologicalElementKind::Edge,
                );
            }
            // Named edges: an evenly sized finish between two program faces.
            let named = EdgeRef::all_named(edges)
                .ok_or(CanonicalError::InvalidTopologicalFeatureReference)?;
            if !fillet_radius_stations.is_empty()
                || !matches!(chamfer_mode, ChamferMode::Symmetric)
                || !chamfer_edge_sides.is_empty()
                || named.iter().any(|edge| {
                    edge.first == edge.second || !edge.first.is_valid() || !edge.second.is_valid()
                })
            {
                return Err(CanonicalError::InvalidTopologicalFeatureReference);
            }
            Ok(())
        }
        FeatureKind::FaceOffset { face, distance, .. } => {
            Dimension::new(distance.source_token(), distance.millimetres()).map(|_| ())?;
            if distance.millimetres().abs() <= PROFILE_EPSILON_MM {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            match face {
                FaceRef::Topological(face) => validate_topological_feature_references(
                    std::slice::from_ref(face.as_ref()),
                    TopologicalElementKind::Face,
                ),
                FaceRef::Named(face) if face.is_valid() => Ok(()),
                FaceRef::Named(_) => Err(CanonicalError::InvalidTopologicalFeatureReference),
            }
        }
        FeatureKind::ImportedExactBody(spec) => validate_imported_exact_body(spec),
        FeatureKind::RigidTransform { transform, .. } => {
            transform
                .rigid_inverse()
                .ok_or(CanonicalError::InvalidTransform)?;
            if transform.matrix()[3].abs() > MAX_COORDINATE_MM
                || transform.matrix()[7].abs() > MAX_COORDINATE_MM
                || transform.matrix()[11].abs() > MAX_COORDINATE_MM
            {
                return Err(CanonicalError::InvalidTransform);
            }
            Ok(())
        }
        FeatureKind::MeshBody(spec) => validate_mesh_body(spec),
        FeatureKind::Revolve {
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            ..
        } => {
            if axis_start_mm
                .iter()
                .chain(axis_end_mm)
                .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
                || (axis_end_mm[0] - axis_start_mm[0]).hypot(axis_end_mm[1] - axis_start_mm[1])
                    <= PROFILE_EPSILON_MM
                || !angle_degrees.is_finite()
                || *angle_degrees <= 0.0
                || *angle_degrees > 360.0
            {
                return Err(CanonicalError::InvalidRevolve);
            }
            Ok(())
        }
        FeatureKind::PlanarOffset { distance, .. } => {
            Dimension::new(distance.source_token(), distance.millimetres()).map(|_| ())?;
            if distance.millimetres().abs() < EXACT_MIN_LENGTH_MM {
                return Err(CanonicalError::InvalidPlanarOffset);
            }
            Ok(())
        }
        FeatureKind::SurfaceExtend { distance, .. } => {
            Dimension::new(distance.source_token(), distance.millimetres()).map(|_| ())?;
            if distance.millimetres() < EXACT_MIN_LENGTH_MM {
                return Err(CanonicalError::InvalidPlanarOffset);
            }
            Ok(())
        }
        FeatureKind::SurfaceTrim { target, cutter } => {
            if target == cutter {
                return Err(CanonicalError::InvalidFeatureOwnership(*target));
            }
            Ok(())
        }
        FeatureKind::SurfaceThicken { thickness, .. } => {
            Dimension::new(thickness.source_token(), thickness.millimetres()).map(|_| ())?;
            if !(EXACT_MIN_LENGTH_MM..=MAX_EXACT_PLANAR_OFFSET_LENGTH_MM)
                .contains(&thickness.millimetres())
            {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            Ok(())
        }
        FeatureKind::SurfaceKnit {
            surfaces,
            tolerance,
            ..
        } => {
            Dimension::new(tolerance.source_token(), tolerance.millimetres()).map(|_| ())?;
            if !(2..=256).contains(&surfaces.len())
                || !surfaces.windows(2).all(|pair| pair[0] < pair[1])
                || !(tolerance_mm..=10.0).contains(&tolerance.millimetres())
            {
                return Err(CanonicalError::InvalidFeatureMap);
            }
            Ok(())
        }
        FeatureKind::Sweep { profile, path } => {
            if profile == path {
                return Err(CanonicalError::InvalidSweep);
            }
            Ok(())
        }
        FeatureKind::WeldmentMember(spec) => {
            if spec.profile == spec.path
                || !spec.orientation_degrees.is_finite()
                || !(-180.0..180.0).contains(&spec.orientation_degrees)
            {
                return Err(CanonicalError::InvalidWeldmentMember);
            }
            Ok(())
        }
        FeatureKind::WeldmentJoint(spec) => {
            if spec.first_member == spec.second_member {
                return Err(CanonicalError::InvalidWeldmentJoint);
            }
            Ok(())
        }
        FeatureKind::SheetMetal(spec) => spec.validate().map_err(CanonicalError::InvalidSheetMetal),
        FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { .. }) => Ok(()),
        FeatureKind::Loft {
            sections,
            guide,
            continuity,
        }
        | FeatureKind::SurfaceBody(SurfaceBodySpec::Loft {
            sections,
            guide,
            continuity,
        }) => {
            if !(2..=16).contains(&sections.len())
                || (guide.is_some() && *continuity == LoftContinuity::Curvature)
                || sections.windows(2).any(|pair| {
                    pair[0].elevation_mm >= pair[1].elevation_mm
                        || pair[0].profile == pair[1].profile
                })
                || sections.iter().any(|section| {
                    !section.elevation_mm.is_finite()
                        || section.elevation_mm.abs() > MAX_COORDINATE_MM
                })
                || sections
                    .iter()
                    .map(|section| section.profile)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != sections.len()
            {
                return Err(CanonicalError::InvalidLoft);
            }
            Ok(())
        }
        FeatureKind::Boolean { .. } => Ok(()),
    }
}

pub(super) fn validate_imported_exact_body(
    spec: &ImportedExactBodySpec,
) -> Result<(), CanonicalError> {
    let bounds_valid = spec
        .bounds_mm
        .iter()
        .flatten()
        .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE_MM)
        && (0..3).all(|axis| spec.bounds_mm[0][axis] <= spec.bounds_mm[1][axis]);
    let legacy_solid = matches!(
        (spec.schema.as_str(), spec.source_part_index, spec.body_kind),
        (IMPORTED_EXACT_BODY_SCHEMA_V1, None, BodyKind::Solid)
            | (
                IMPORTED_EXACT_BODY_SCHEMA_V2,
                Some(0..=1_023),
                BodyKind::Solid
            )
    );
    let typed_body = spec.schema == IMPORTED_EXACT_BODY_SCHEMA_V3
        && spec.source_part_index.is_none_or(|index| index <= 1_023);
    let measurements_valid = match spec.body_kind {
        BodyKind::Solid => {
            (1..=1_024).contains(&spec.solid_count)
                && spec.volume_mm3.is_finite()
                && spec.volume_mm3 > 0.0
                && (legacy_solid || (spec.area_mm2.is_finite() && spec.area_mm2 > 0.0))
        }
        BodyKind::Surface => {
            typed_body
                && spec.solid_count == 0
                && spec.volume_mm3.is_finite()
                && spec.volume_mm3.abs() <= APPROXIMATION
                && spec.area_mm2.is_finite()
                && spec.area_mm2 > 0.0
        }
    };
    let reject = |reason| Err(CanonicalError::InvalidImportReceipt(reason));
    if spec.import_id.0 == 0 {
        return reject(ImportContractError::InvalidIdentity);
    }
    if (!legacy_solid && !typed_body)
        || spec.source_byte_len == 0
        || spec.source_byte_len > 32 * 1024 * 1024
        || spec.source_sha256.iter().all(|byte| *byte == 0)
    {
        return reject(ImportContractError::InvalidSource);
    }
    if spec.result_fingerprint.is_empty()
        || spec.result_fingerprint.len() > 128
        || spec.backend.is_empty()
        || spec.backend.len() > 1_024
        || spec.tolerance.is_empty()
        || spec.tolerance.len() > 1_024
    {
        return reject(ImportContractError::InvalidText);
    }
    if spec.topology_counts.is_some_and(|counts| {
        counts[..3].contains(&0)
            || counts[4] != spec.solid_count
            || counts[..3]
                .iter()
                .map(|count| u64::from(*count))
                .sum::<u64>()
                > crate::topology::MAX_GENERATED_TOPOLOGICAL_REFERENCES
    }) || !measurements_valid
        || !bounds_valid
    {
        return reject(ImportContractError::InvalidEvidence);
    }
    Ok(())
}

pub(super) const MAX_MESH_VERTICES: usize = 100_000;
pub(super) const MAX_MESH_TRIANGLES: usize = 200_000;
pub(super) const MESH_AREA_EPSILON: f64 = ROUNDING * ROUNDING;
pub(super) const MESH_VOLUME_EPSILON: f64 = APPROXIMATION;

pub(super) fn validate_mesh_body(spec: &MeshBodySpec) -> Result<(), CanonicalError> {
    if spec.schema != MESH_BODY_SCHEMA_V1
        || !(4..=MAX_MESH_VERTICES).contains(&spec.vertices_mm.len())
        || !(4..=MAX_MESH_TRIANGLES).contains(&spec.triangles.len())
        || spec
            .vertices_mm
            .iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE_MM)
    {
        return Err(CanonicalError::InvalidMeshBody);
    }
    match &spec.authority {
        MeshAuthority::Authored { provenance } if provenance.is_empty() => {
            return Err(CanonicalError::InvalidMeshBody);
        }
        MeshAuthority::ImportedStl { import_id }
        | MeshAuthority::ImportedSketchupScene { import_id }
        | MeshAuthority::ImportedGlb { import_id }
            if import_id.0 == 0 =>
        {
            return Err(CanonicalError::InvalidMeshBody);
        }
        MeshAuthority::ExactConversion(conversion)
            if conversion.source_document_id.0 == 0
                || conversion.source_revision == 0
                || conversion.source_digest.is_empty()
                || conversion.source_definition_id.0 == 0
                || conversion.source_feature_id.0 == 0
                || conversion.source_result_fingerprint.is_empty()
                || conversion.source_evaluator.is_empty()
                || conversion.source_backend.is_empty()
                || conversion.source_tolerance.is_empty()
                || conversion.tessellation_tolerance.is_empty()
                || conversion.destination_definition_id.0 == 0
                || conversion.destination_feature_id.0 == 0
                || conversion.unsupported_semantics.is_empty()
                || !conversion
                    .unsupported_semantics
                    .windows(2)
                    .all(|pair| pair[0] < pair[1]) =>
        {
            return Err(CanonicalError::InvalidMeshBody);
        }
        _ => {}
    }

    let mut edges = BTreeMap::<(u32, u32), (u32, i32, Vec<usize>)>::new();
    let mut seen_triangles = BTreeSet::new();
    let volume_origin = spec.vertices_mm[0];
    let mut signed_volume_times_six = 0.0;
    let mut volume_compensation = 0.0;
    for (triangle_index, indices) in spec.triangles.iter().enumerate() {
        let [a, b, c] = *indices;
        if a == b
            || b == c
            || a == c
            || [a, b, c]
                .into_iter()
                .any(|index| index as usize >= spec.vertices_mm.len())
        {
            return Err(CanonicalError::InvalidMeshBody);
        }
        let mut canonical = [a, b, c];
        canonical.sort_unstable();
        if !seen_triangles.insert(canonical) {
            return Err(CanonicalError::InvalidMeshBody);
        }
        let first = spec.vertices_mm[a as usize];
        let second = spec.vertices_mm[b as usize];
        let third = spec.vertices_mm[c as usize];
        let first_edge = [
            second[0] - first[0],
            second[1] - first[1],
            second[2] - first[2],
        ];
        let second_edge = [
            third[0] - first[0],
            third[1] - first[1],
            third[2] - first[2],
        ];
        let cross = ketchup_geometry::linalg::cross(first_edge, second_edge);
        if cross.into_iter().map(|value| value * value).sum::<f64>() <= MESH_AREA_EPSILON {
            return Err(CanonicalError::InvalidMeshBody);
        }
        let shifted = [first, second, third].map(|point| {
            [
                point[0] - volume_origin[0],
                point[1] - volume_origin[1],
                point[2] - volume_origin[2],
            ]
        });
        let volume_term = shifted[0][0]
            * (shifted[1][1] * shifted[2][2] - shifted[1][2] * shifted[2][1])
            + shifted[0][1] * (shifted[1][2] * shifted[2][0] - shifted[1][0] * shifted[2][2])
            + shifted[0][2] * (shifted[1][0] * shifted[2][1] - shifted[1][1] * shifted[2][0]);
        let corrected = volume_term - volume_compensation;
        let next = signed_volume_times_six + corrected;
        volume_compensation = (next - signed_volume_times_six) - corrected;
        signed_volume_times_six = next;
        for (from, to) in [(a, b), (b, c), (c, a)] {
            let key = (from.min(to), from.max(to));
            let entry = edges.entry(key).or_default();
            entry.0 += 1;
            entry.1 += if from < to { 1 } else { -1 };
            entry.2.push(triangle_index);
        }
    }
    if edges
        .values()
        .any(|(uses, orientation, _)| *uses != 2 || *orientation != 0)
        || !mesh_vertex_fans_are_manifold(spec.vertices_mm.len(), &spec.triangles, &edges)
        || signed_volume_times_six <= MESH_VOLUME_EPSILON
    {
        return Err(CanonicalError::InvalidMeshBody);
    }
    Ok(())
}

pub(super) fn mesh_vertex_fans_are_manifold(
    vertex_count: usize,
    triangles: &[[u32; 3]],
    edges: &BTreeMap<(u32, u32), (u32, i32, Vec<usize>)>,
) -> bool {
    let mut incident = vec![BTreeSet::new(); vertex_count];
    let mut adjacency = vec![BTreeMap::<usize, BTreeSet<usize>>::new(); vertex_count];
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        for vertex in triangle {
            incident[*vertex as usize].insert(triangle_index);
        }
    }
    for ((first, second), (_, _, uses)) in edges {
        if let [left, right] = uses.as_slice() {
            for vertex in [*first, *second] {
                adjacency[vertex as usize]
                    .entry(*left)
                    .or_default()
                    .insert(*right);
                adjacency[vertex as usize]
                    .entry(*right)
                    .or_default()
                    .insert(*left);
            }
        }
    }
    incident.iter().enumerate().all(|(vertex, faces)| {
        let Some(start) = faces.first().cloned() else {
            return false;
        };
        if faces.iter().any(|face| {
            adjacency[vertex]
                .get(face)
                .is_none_or(|neighbours| neighbours.len() != 2)
        }) {
            return false;
        }
        let mut visited = BTreeSet::new();
        let mut pending = vec![start];
        while let Some(face) = pending.pop() {
            if visited.insert(face)
                && let Some(neighbours) = adjacency[vertex].get(&face)
            {
                pending.extend(neighbours.iter().cloned());
            }
        }
        &visited == faces
    })
}

pub(super) const MAX_PROFILE_POINTS: usize = 1_024;
pub(super) const PROFILE_EPSILON_MM: f64 = ROUNDING;

pub(super) fn is_axis_aligned_rectangle(points_mm: &[[f64; 2]]) -> bool {
    points_mm.len() == 4
        && points_mm[0][1] == points_mm[1][1]
        && points_mm[1][0] == points_mm[2][0]
        && points_mm[2][1] == points_mm[3][1]
        && points_mm[3][0] == points_mm[0][0]
        && points_mm[1][0] > points_mm[0][0]
        && points_mm[3][1] > points_mm[0][1]
}

pub(super) fn resize_axis_aligned_rectangle(
    points_mm: &[[f64; 2]],
    target: &FeatureParameterTarget,
    value_mm: f64,
    tolerance_mm: f64,
) -> Result<Vec<[f64; 2]>, CanonicalError> {
    if !is_axis_aligned_rectangle(points_mm) {
        return Err(CanonicalError::InvalidFeatureParameterBinding(
            target.clone(),
        ));
    }
    if value_mm <= PROFILE_EPSILON_MM {
        return Err(CanonicalError::DimensionOutsideEnvelope);
    }
    let mut resized = points_mm.to_vec();
    match target.path.as_str() {
        "bounds.width" => {
            let right = points_mm[0][0] + value_mm;
            resized[1][0] = right;
            resized[2][0] = right;
        }
        "bounds.height" => {
            let top = points_mm[0][1] + value_mm;
            resized[2][1] = top;
            resized[3][1] = top;
        }
        _ => {
            return Err(CanonicalError::InvalidFeatureParameterBinding(
                target.clone(),
            ));
        }
    }
    validate_feature_kind(&FeatureKind::polygon(&resized), tolerance_mm)?;
    Ok(resized)
}

pub(super) fn sweep_path_segment_metrics(
    segment: &ProfileSegment,
    tolerance_mm: f64,
) -> Option<(f64, [f64; 2], [f64; 2])> {
    match segment {
        // The exact kernel sweeps along lines, arcs and Bezier curves only.
        ProfileSegment::Spline { .. } => None,
        ProfileSegment::Line { start_mm, end_mm } => {
            let direction = [end_mm[0] - start_mm[0], end_mm[1] - start_mm[1]];
            let length = direction[0].hypot(direction[1]);
            if !length.is_finite() || length <= tolerance_mm {
                return None;
            }
            let tangent = [direction[0] / length, direction[1] / length];
            Some((length, tangent, tangent))
        }
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => {
            let start_radius = [start_mm[0] - center_mm[0], start_mm[1] - center_mm[1]];
            let end_radius = [end_mm[0] - center_mm[0], end_mm[1] - center_mm[1]];
            let radius = start_radius[0].hypot(start_radius[1]);
            let end_radius_length = end_radius[0].hypot(end_radius[1]);
            let start_angle = start_radius[1].atan2(start_radius[0]);
            let end_angle = end_radius[1].atan2(end_radius[0]);
            let sweep_angle = if *clockwise {
                (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
            } else {
                (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
            };
            if !radius.is_finite()
                || radius <= tolerance_mm
                || (radius - end_radius_length).abs() > PROFILE_EPSILON_MM
                || start_mm == end_mm
                || !sweep_angle.is_finite()
                || radius * sweep_angle <= tolerance_mm
            {
                return None;
            }
            let tangent = |radial: [f64; 2], radial_length: f64| {
                if *clockwise {
                    [radial[1] / radial_length, -radial[0] / radial_length]
                } else {
                    [-radial[1] / radial_length, radial[0] / radial_length]
                }
            };
            Some((
                radius * sweep_angle,
                tangent(start_radius, radius),
                tangent(end_radius, end_radius_length),
            ))
        }
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => {
            let forward = ketchup_geometry::linalg::CubicBezier::new([
                *start_mm,
                *control_1_mm,
                *control_2_mm,
                *end_mm,
            ])
            .forward(tolerance_mm)?;
            Some((
                forward.control_length,
                forward.start_tangent,
                forward.end_tangent,
            ))
        }
    }
}

impl ProfileSegment {
    /// The segments that trace a sketch entity from its start; a circle is two half arcs
    /// starting at its point of least x.
    #[must_use]
    pub fn from_sketch_entity(entity: &SketchEntity) -> Vec<Self> {
        match entity {
            SketchEntity::Line {
                start_mm, end_mm, ..
            } => vec![Self::Line {
                start_mm: *start_mm,
                end_mm: *end_mm,
            }],
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
                ..
            } => vec![Self::CircularArc {
                start_mm: *start_mm,
                end_mm: *end_mm,
                center_mm: *center_mm,
                clockwise: *clockwise,
            }],
            SketchEntity::Circle {
                center_mm,
                radius_mm,
                ..
            } => {
                let left = [center_mm[0] - radius_mm, center_mm[1]];
                let right = [center_mm[0] + radius_mm, center_mm[1]];
                vec![
                    Self::CircularArc {
                        start_mm: left,
                        end_mm: right,
                        center_mm: *center_mm,
                        clockwise: false,
                    },
                    Self::CircularArc {
                        start_mm: right,
                        end_mm: left,
                        center_mm: *center_mm,
                        clockwise: false,
                    },
                ]
            }
            SketchEntity::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => vec![Self::CubicBezier {
                start_mm: *start_mm,
                control_1_mm: *control_1_mm,
                control_2_mm: *control_2_mm,
                end_mm: *end_mm,
            }],
        }
    }

    /// `[min, max]` around the segment: an arc counts as its whole circle and a curve as
    /// the points that define it.
    #[must_use]
    pub fn bounds_mm(&self) -> [[f64; 2]; 2] {
        let segment = self;
        let points = match segment {
            ProfileSegment::Line { start_mm, end_mm } => vec![*start_mm, *end_mm],
            ProfileSegment::CircularArc {
                start_mm,
                center_mm,
                ..
            } => {
                let start_radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                let end_mm = segment.end_mm();
                let end_radius = (end_mm[0] - center_mm[0]).hypot(end_mm[1] - center_mm[1]);
                let radius = start_radius.max(end_radius);
                return [
                    [center_mm[0] - radius, center_mm[1] - radius],
                    [center_mm[0] + radius, center_mm[1] + radius],
                ];
            }
            ProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => vec![*start_mm, *control_1_mm, *control_2_mm, *end_mm],
            ProfileSegment::Spline { points_mm } => points_mm.clone(),
        };
        [0, 1].map(|bound| {
            [0, 1].map(|axis| {
                points.iter().fold(
                    if bound == 0 {
                        f64::INFINITY
                    } else {
                        f64::NEG_INFINITY
                    },
                    |value, point| {
                        if bound == 0 {
                            value.min(point[axis])
                        } else {
                            value.max(point[axis])
                        }
                    },
                )
            })
        })
    }
}

pub(super) fn sweep_path_arc_angle(segment: &ProfileSegment) -> Option<f64> {
    let ProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm,
        clockwise,
    } = segment
    else {
        return None;
    };
    let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
    let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
    Some(if *clockwise {
        (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    })
}

pub(super) fn sweep_path_join_is_separated(
    left: &ProfileSegment,
    right: &ProfileSegment,
    tangent: [f64; 2],
) -> bool {
    let join = left.end_mm();
    let projection =
        |point: [f64; 2]| (point[0] - join[0]) * tangent[0] + (point[1] - join[1]) * tangent[1];
    let left_is_behind = match left {
        ProfileSegment::Line { start_mm, .. } => projection(*start_mm) < -PROFILE_EPSILON_MM,
        ProfileSegment::CircularArc { .. } => sweep_path_arc_angle(left)
            .is_some_and(|angle| angle < std::f64::consts::PI - PROFILE_EPSILON_MM),
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            ..
        } => [*start_mm, *control_1_mm, *control_2_mm]
            .into_iter()
            .all(|point| projection(point) < -PROFILE_EPSILON_MM),
        ProfileSegment::Spline { .. } => false,
    };
    let right_is_ahead = match right {
        ProfileSegment::Line { end_mm, .. } => projection(*end_mm) > PROFILE_EPSILON_MM,
        ProfileSegment::CircularArc { .. } => sweep_path_arc_angle(right)
            .is_some_and(|angle| angle < std::f64::consts::PI - PROFILE_EPSILON_MM),
        ProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } => [*control_1_mm, *control_2_mm, *end_mm]
            .into_iter()
            .all(|point| projection(point) > PROFILE_EPSILON_MM),
        ProfileSegment::Spline { .. } => false,
    };
    left_is_behind && right_is_ahead
}

pub(super) fn sweep_path_self_intersects(
    segments: &[ProfileSegment],
    metrics: &[(f64, [f64; 2], [f64; 2])],
) -> bool {
    if segments
        .windows(2)
        .zip(metrics.windows(2))
        .any(|(segments, metrics)| {
            !sweep_path_join_is_separated(&segments[0], &segments[1], metrics[0].2)
        })
    {
        return true;
    }
    let bounds = segments
        .iter()
        .map(ProfileSegment::bounds_mm)
        .collect::<Vec<_>>();
    for left in 0..bounds.len() {
        for right in left + 2..bounds.len() {
            if [0, 1].into_iter().all(|axis| {
                bounds[left][0][axis] <= bounds[right][1][axis] + PROFILE_EPSILON_MM
                    && bounds[right][0][axis] <= bounds[left][1][axis] + PROFILE_EPSILON_MM
            }) {
                return true;
            }
        }
    }
    false
}

pub fn solved_sketch_sweep_path(
    sketch: &SketchSpec,
    tolerance_mm: f64,
) -> Option<Vec<ProfileSegment>> {
    let solution = sketch.solve_geometry().ok()?;
    let segments = solution
        .entities
        .iter()
        .map(|entity| {
            (!matches!(entity, SketchEntity::Circle { .. }))
                .then(|| ProfileSegment::from_sketch_entity(entity))
        })
        .collect::<Option<Vec<_>>>()?
        .concat();
    is_valid_sweep_path(&segments, tolerance_mm).then_some(segments)
}

#[derive(Clone, Copy, Debug)]
pub struct ValidatedSweepProfile<'a>(pub(super) ValidatedSweepProfileKind<'a>);

#[derive(Clone, Copy, Debug)]
pub(super) enum ValidatedSweepProfileKind<'a> {
    LineArcBoundary(&'a [ProfileSegment]),
    Sketch(&'a SketchSpec),
}

impl<'a> ValidatedSweepProfile<'a> {
    #[must_use]
    pub fn line_arc_boundary(self) -> Option<&'a [ProfileSegment]> {
        match self.0 {
            ValidatedSweepProfileKind::LineArcBoundary(segments) => Some(segments),
            _ => None,
        }
    }

    #[must_use]
    pub fn sketch(self) -> Option<&'a SketchSpec> {
        match self.0 {
            ValidatedSweepProfileKind::Sketch(sketch) => Some(sketch),
            _ => None,
        }
    }

    #[must_use]
    pub fn from_feature_kind(kind: &'a FeatureKind, tolerance: TolerancePolicy) -> Option<Self> {
        match kind {
            FeatureKind::Profile {
                segments,
                closed: true,
            } if accepts_sweep_segment_profile(segments, true, tolerance)
                || (kind.polygon_points().is_some()
                    && sweep_profile_is_valid(kind, tolerance.linear_mm())) =>
            {
                Some(Self(ValidatedSweepProfileKind::LineArcBoundary(segments)))
            }
            FeatureKind::Sketch(sketch) if valid_sketch_sweep_profile(sketch) => {
                Some(Self(ValidatedSweepProfileKind::Sketch(sketch)))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ValidatedSweepPath<'a>(pub(super) ValidatedSweepPathKind<'a>);

#[derive(Clone, Copy, Debug)]
pub(super) enum ValidatedSweepPathKind<'a> {
    Planar(&'a [ProfileSegment]),
    Spatial(&'a [SpatialPathSegment]),
    Sketch(&'a SketchSpec),
}

impl<'a> ValidatedSweepPath<'a> {
    #[must_use]
    pub fn planar(self) -> Option<&'a [ProfileSegment]> {
        match self.0 {
            ValidatedSweepPathKind::Planar(segments) => Some(segments),
            _ => None,
        }
    }

    #[must_use]
    pub fn spatial(self) -> Option<&'a [SpatialPathSegment]> {
        match self.0 {
            ValidatedSweepPathKind::Spatial(segments) => Some(segments),
            _ => None,
        }
    }

    #[must_use]
    pub fn sketch(self) -> Option<&'a SketchSpec> {
        match self.0 {
            ValidatedSweepPathKind::Sketch(sketch) => Some(sketch),
            _ => None,
        }
    }

    #[must_use]
    pub fn from_feature_kind(kind: &'a FeatureKind, tolerance_mm: f64) -> Option<Self> {
        match kind {
            FeatureKind::Profile {
                segments,
                closed: false,
            } if is_valid_sweep_path(segments, tolerance_mm) => {
                Some(Self(ValidatedSweepPathKind::Planar(segments)))
            }
            FeatureKind::SpatialPath { segments }
                if is_valid_spatial_sweep_path(segments, tolerance_mm) =>
            {
                Some(Self(ValidatedSweepPathKind::Spatial(segments)))
            }
            FeatureKind::Sketch(sketch)
                if solved_sketch_sweep_path(sketch, tolerance_mm).is_some() =>
            {
                Some(Self(ValidatedSweepPathKind::Sketch(sketch)))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ValidatedSweepInputs<'a> {
    pub(super) profile: ValidatedSweepProfile<'a>,
    pub(super) path: ValidatedSweepPath<'a>,
}

impl<'a> ValidatedSweepInputs<'a> {
    #[must_use]
    pub fn profile(self) -> ValidatedSweepProfile<'a> {
        self.profile
    }

    #[must_use]
    pub fn path(self) -> ValidatedSweepPath<'a> {
        self.path
    }

    #[must_use]
    pub fn from_snapshot(
        snapshot: &Snapshot,
        profile_kind: &'a FeatureKind,
        path_kind: &'a FeatureKind,
    ) -> Option<Self> {
        Self::with_workplane_frames(snapshot.tolerance(), profile_kind, path_kind, |workplane| {
            snapshot.feature(workplane).and_then(|feature| {
                if let FeatureKind::Workplane(spec) = feature.kind() {
                    Some(spec.frame)
                } else {
                    None
                }
            })
        })
    }

    pub(super) fn with_workplane_frames(
        tolerance: TolerancePolicy,
        profile_kind: &'a FeatureKind,
        path_kind: &'a FeatureKind,
        frame: impl FnMut(FeatureId) -> Option<WorkplaneFrame>,
    ) -> Option<Self> {
        let profile = ValidatedSweepProfile::from_feature_kind(profile_kind, tolerance)?;
        let path = ValidatedSweepPath::from_feature_kind(path_kind, tolerance.linear_mm())?;
        let compatible = match (profile.0, path.0) {
            (ValidatedSweepProfileKind::Sketch(profile), ValidatedSweepPathKind::Sketch(path)) => {
                valid_sketch_sweep_inputs_with_frames(profile, path, frame, tolerance.linear_mm())
            }
            (ValidatedSweepProfileKind::Sketch(profile), ValidatedSweepPathKind::Spatial(path)) => {
                valid_sketch_spatial_sweep_inputs_with_frame(
                    profile,
                    path,
                    frame,
                    tolerance.linear_mm(),
                )
            }
            (ValidatedSweepProfileKind::Sketch(_), _) | (_, ValidatedSweepPathKind::Sketch(_)) => {
                false
            }
            (_, ValidatedSweepPathKind::Spatial(path)) => {
                spatial_sweep_bounds_are_valid(profile_kind, path, tolerance.linear_mm())
            }
            _ => true,
        };
        compatible.then_some(Self { profile, path })
    }
}

pub fn valid_sketch_sweep_profile(profile: &SketchSpec) -> bool {
    let Ok(regions) = profile.solved_regions() else {
        return false;
    };
    let [region] = regions.as_slice() else {
        return false;
    };
    match &region.outer {
        SolvedSketchRegionProfile::Polyline(points) => {
            (3..=MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS).contains(&points.len())
        }
        SolvedSketchRegionProfile::Boundary(edges) => {
            (2..=MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS).contains(&edges.len())
        }
        SolvedSketchRegionProfile::Circle { .. } => true,
    }
}

pub(super) fn valid_sketch_spatial_sweep_inputs_with_frame(
    profile: &SketchSpec,
    path: &[SpatialPathSegment],
    mut frame: impl FnMut(FeatureId) -> Option<WorkplaneFrame>,
    tolerance_mm: f64,
) -> bool {
    if !valid_sketch_sweep_profile(profile) || !is_valid_spatial_sweep_path(path, tolerance_mm) {
        return false;
    }
    let Some(profile_frame) = frame(profile.workplane) else {
        return false;
    };
    let start = path[0].start_mm();
    let tangent = match &path[0] {
        SpatialPathSegment::Line { start_mm, end_mm } => {
            [0, 1, 2].map(|axis| end_mm[axis] - start_mm[axis])
        }
        SpatialPathSegment::CircularArc {
            start_mm,
            center_mm,
            normal,
            clockwise,
            ..
        } => {
            let radius = [0, 1, 2].map(|axis| start_mm[axis] - center_mm[axis]);
            let cross = ketchup_geometry::linalg::cross(*normal, radius);
            cross.map(|value| if *clockwise { -value } else { value })
        }
        SpatialPathSegment::CubicBezier {
            start_mm,
            control_1_mm,
            ..
        } => [0, 1, 2].map(|axis| control_1_mm[axis] - start_mm[axis]),
    };
    let length = tangent[0].hypot(tangent[1]).hypot(tangent[2]);
    let direction = tangent.map(|component| component / length);
    [0, 1, 2]
        .into_iter()
        .all(|axis| (start[axis] - profile_frame.origin_mm[axis]).abs() <= ROUNDING)
        && [0, 1, 2]
            .into_iter()
            .map(|axis| direction[axis] * profile_frame.normal[axis])
            .sum::<f64>()
            >= 1.0 - ROUNDING
}

pub(super) fn valid_sketch_sweep_inputs_with_frames(
    profile: &SketchSpec,
    path: &SketchSpec,
    mut frame: impl FnMut(FeatureId) -> Option<WorkplaneFrame>,
    tolerance_mm: f64,
) -> bool {
    if !valid_sketch_sweep_profile(profile) {
        return false;
    }
    let Some(path_segments) = solved_sketch_sweep_path(path, tolerance_mm) else {
        return false;
    };
    let Some((_, start_tangent, _)) = sweep_path_segment_metrics(&path_segments[0], tolerance_mm)
    else {
        return false;
    };
    let start_mm = path_segments[0].start_mm();
    let (Some(profile_frame), Some(path_frame)) = (frame(profile.workplane), frame(path.workplane))
    else {
        return false;
    };
    let start = [0, 1, 2].map(|axis| {
        path_frame.origin_mm[axis]
            + path_frame.x_axis[axis] * start_mm[0]
            + path_frame.y_axis[axis] * start_mm[1]
    });
    let direction = [0, 1, 2].map(|axis| {
        path_frame.x_axis[axis] * start_tangent[0] + path_frame.y_axis[axis] * start_tangent[1]
    });
    let starts_at_profile = [0, 1, 2]
        .into_iter()
        .all(|axis| (start[axis] - profile_frame.origin_mm[axis]).abs() <= ROUNDING);
    let aligned = [0, 1, 2]
        .into_iter()
        .map(|axis| direction[axis] * profile_frame.normal[axis])
        .sum::<f64>()
        >= 1.0 - ROUNDING;
    starts_at_profile && aligned
}

pub fn is_valid_sweep_path(segments: &[ProfileSegment], tolerance_mm: f64) -> bool {
    if !(1..=MAX_EXACT_BREP_SWEEP_PATH_SEGMENTS).contains(&segments.len())
        || segments.len() == 1 && !matches!(segments[0], ProfileSegment::Line { .. })
    {
        return false;
    }
    let metrics = segments
        .iter()
        .map(|segment| sweep_path_segment_metrics(segment, tolerance_mm))
        .collect::<Option<Vec<_>>>();
    let Some(metrics) = metrics else {
        return false;
    };
    let total_length = metrics.iter().map(|metrics| metrics.0).sum::<f64>();
    if !(MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM..=MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM)
        .contains(&total_length)
        || sweep_path_self_intersects(segments, &metrics)
    {
        return false;
    }
    metrics.windows(2).all(|pair| {
        let outgoing = pair[0].2;
        let incoming = pair[1].1;
        let dot = ketchup_geometry::linalg::dot2(outgoing, incoming);
        let cross = ketchup_geometry::linalg::cross2(outgoing, incoming);
        dot >= 1.0 - ROUNDING && cross.abs() <= ROUNDING
    })
}

pub fn is_valid_spatial_sweep_path(segments: &[SpatialPathSegment], tolerance_mm: f64) -> bool {
    use ketchup_geometry::linalg::{cross, dot, length, normalize_within as unit, sub};
    fn arc_angle(segment: &SpatialPathSegment, tolerance_mm: f64) -> Option<f64> {
        let SpatialPathSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            normal,
            clockwise,
        } = segment
        else {
            return None;
        };
        let normal = unit(*normal, tolerance_mm)?;
        let start_radius = sub(*start_mm, *center_mm);
        let end_radius = sub(*end_mm, *center_mm);
        let signed =
            dot(normal, cross(start_radius, end_radius)).atan2(dot(start_radius, end_radius));
        Some(if *clockwise {
            (-signed).rem_euclid(std::f64::consts::TAU)
        } else {
            signed.rem_euclid(std::f64::consts::TAU)
        })
    }
    fn join_is_separated(
        left: &SpatialPathSegment,
        right: &SpatialPathSegment,
        tangent: [f64; 3],
        tolerance_mm: f64,
    ) -> bool {
        let join = left.end_mm();
        let projection = |point: [f64; 3]| dot(sub(point, join), tangent);
        let left_is_behind = match left {
            SpatialPathSegment::Line { start_mm, .. } => {
                projection(*start_mm) < -PROFILE_EPSILON_MM
            }
            SpatialPathSegment::CircularArc { .. } => arc_angle(left, tolerance_mm)
                .is_some_and(|angle| angle < std::f64::consts::PI - PROFILE_EPSILON_MM),
            SpatialPathSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                ..
            } => [*start_mm, *control_1_mm, *control_2_mm]
                .into_iter()
                .all(|point| projection(point) < -PROFILE_EPSILON_MM),
        };
        let right_is_ahead = match right {
            SpatialPathSegment::Line { end_mm, .. } => projection(*end_mm) > PROFILE_EPSILON_MM,
            SpatialPathSegment::CircularArc { .. } => arc_angle(right, tolerance_mm)
                .is_some_and(|angle| angle < std::f64::consts::PI - PROFILE_EPSILON_MM),
            SpatialPathSegment::CubicBezier {
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => [*control_1_mm, *control_2_mm, *end_mm]
                .into_iter()
                .all(|point| projection(point) > PROFILE_EPSILON_MM),
        };
        left_is_behind && right_is_ahead
    }
    fn metrics(
        segment: &SpatialPathSegment,
        tolerance_mm: f64,
    ) -> Option<(f64, [f64; 3], [f64; 3])> {
        let start = segment.start_mm();
        let end = segment.end_mm();
        if [start, end]
            .into_iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE_MM)
        {
            return None;
        }
        match segment {
            SpatialPathSegment::Line { .. } => {
                let direction = sub(end, start);
                let tangent = unit(direction, tolerance_mm)?;
                Some((length(direction), tangent, tangent))
            }
            SpatialPathSegment::CircularArc {
                center_mm,
                normal,
                clockwise,
                ..
            } => {
                if [*center_mm, *normal]
                    .into_iter()
                    .flatten()
                    .any(|coordinate| {
                        !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE_MM
                    })
                {
                    return None;
                }
                let normal_length = length(*normal);
                if (normal_length - 1.0).abs() > PROFILE_EPSILON_MM {
                    return None;
                }
                let normal = unit(*normal, tolerance_mm)?;
                let start_radius = sub(start, *center_mm);
                let end_radius = sub(end, *center_mm);
                let radius = length(start_radius);
                let end_radius_length = length(end_radius);
                if radius <= tolerance_mm
                    || (radius - end_radius_length).abs() > PROFILE_EPSILON_MM
                    || dot(start_radius, normal).abs() > PROFILE_EPSILON_MM
                    || dot(end_radius, normal).abs() > PROFILE_EPSILON_MM
                    || start == end
                {
                    return None;
                }
                let signed = dot(normal, cross(start_radius, end_radius))
                    .atan2(dot(start_radius, end_radius));
                let angle = if *clockwise {
                    (-signed).rem_euclid(std::f64::consts::TAU)
                } else {
                    signed.rem_euclid(std::f64::consts::TAU)
                };
                if radius * angle <= tolerance_mm {
                    return None;
                }
                let sign = if *clockwise { -1.0 } else { 1.0 };
                let start_tangent =
                    unit(cross(normal, start_radius).map(|v| sign * v), tolerance_mm)?;
                let end_tangent = unit(cross(normal, end_radius).map(|v| sign * v), tolerance_mm)?;
                Some((radius * angle, start_tangent, end_tangent))
            }
            SpatialPathSegment::CubicBezier {
                control_1_mm,
                control_2_mm,
                ..
            } => {
                if [*control_1_mm, *control_2_mm]
                    .into_iter()
                    .flatten()
                    .any(|coordinate| {
                        !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE_MM
                    })
                {
                    return None;
                }
                let chord = sub(end, start);
                let first = sub(*control_1_mm, start);
                let middle = sub(*control_2_mm, *control_1_mm);
                let last = sub(end, *control_2_mm);
                let chord_squared = dot(chord, chord);
                let first_length = length(first);
                let last_length = length(last);
                let projection_1 = dot(first, chord);
                let projection_2 = dot(sub(*control_2_mm, start), chord);
                if projection_1 <= 0.0
                    || projection_2 < projection_1
                    || projection_2 >= chord_squared
                {
                    return None;
                }
                Some((
                    first_length + length(middle) + last_length,
                    unit(first, tolerance_mm)?,
                    unit(last, tolerance_mm)?,
                ))
            }
        }
    }

    if !(1..=MAX_EXACT_BREP_SWEEP_PATH_SEGMENTS).contains(&segments.len()) {
        return false;
    }
    let Some(metrics) = segments
        .iter()
        .map(|segment| metrics(segment, tolerance_mm))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let total_length = metrics.iter().map(|metric| metric.0).sum::<f64>();
    if !(MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM..=MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM)
        .contains(&total_length)
    {
        return false;
    }
    let closed = segments.first().unwrap().start_mm() == segments.last().unwrap().end_mm();
    if segments
        .windows(2)
        .zip(metrics.windows(2))
        .any(|(segments, metrics)| {
            if segments[0].end_mm() != segments[1].start_mm() {
                return true;
            }
            if let (
                SpatialPathSegment::CircularArc {
                    start_mm,
                    center_mm: left_center,
                    ..
                },
                SpatialPathSegment::CircularArc {
                    center_mm: right_center,
                    ..
                },
            ) = (&segments[0], &segments[1])
                && left_center == right_center
            {
                let radius = length(sub(*start_mm, *left_center));
                if metrics[0].0 + metrics[1].0
                    >= std::f64::consts::TAU * radius - PROFILE_EPSILON_MM
                {
                    return true;
                }
            }
            let outgoing = metrics[0].2;
            let incoming = metrics[1].1;
            !join_is_separated(&segments[0], &segments[1], outgoing, tolerance_mm)
                || dot(outgoing, incoming) < 1.0 - ROUNDING
                || length(cross(outgoing, incoming)) > ROUNDING
        })
    {
        return false;
    }
    if closed {
        let last = segments.last().unwrap();
        let first = segments.first().unwrap();
        let outgoing = metrics.last().unwrap().2;
        let incoming = metrics.first().unwrap().1;
        if !join_is_separated(last, first, outgoing, tolerance_mm)
            || dot(outgoing, incoming) < 1.0 - ROUNDING
            || length(cross(outgoing, incoming)) > ROUNDING
        {
            return false;
        }
    }
    // World-axis AABB overlap is only a broad-phase candidate, not proof that two
    // spatial curves intersect. Arbitrarily oriented helices routinely overlap in
    // all three projected intervals while remaining disjoint. The exact worker
    // performs the authoritative edge-to-edge distance test before any sweep.
    true
}

pub(super) fn is_valid_segment_profile(segments: &[ProfileSegment], closed: bool) -> bool {
    if segments.is_empty() || segments.len() > MAX_PROFILE_POINTS {
        return false;
    }
    let valid_point = |point: [f64; 2]| {
        point
            .into_iter()
            .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE_MM)
    };
    let distinct = |left: [f64; 2], right: [f64; 2]| {
        (left[0] - right[0]).hypot(left[1] - right[1]) > PROFILE_EPSILON_MM
    };
    for segment in segments {
        let points = segment.defining_points_mm();
        if points.len() > MAX_PROFILE_POINTS || !points.iter().copied().all(valid_point) {
            return false;
        }
        let start = segment.start_mm();
        let end = segment.end_mm();
        let spans = match segment {
            // A spline is cubic: it passes through at least four distinct points, each
            // distinct from the one before. Closing on itself, it repeats the first.
            ProfileSegment::Spline { points_mm } => {
                points_mm.windows(2).all(|pair| distinct(pair[0], pair[1]))
                    && points_mm.len() >= if distinct(start, end) { 4 } else { 5 }
            }
            _ => distinct(start, end),
        };
        if !spans {
            return false;
        }
        if let ProfileSegment::CircularArc { center_mm, .. } = segment {
            let start_radius = (start[0] - center_mm[0]).hypot(start[1] - center_mm[1]);
            let end_radius = (end[0] - center_mm[0]).hypot(end[1] - center_mm[1]);
            let radius_tolerance = PROFILE_EPSILON_MM * start_radius.max(end_radius).max(1.0);
            if start_radius <= PROFILE_EPSILON_MM
                || (start_radius - end_radius).abs() > radius_tolerance
            {
                return false;
            }
        }
    }
    if segments
        .windows(2)
        .any(|pair| pair[0].end_mm() != pair[1].start_mm())
    {
        return false;
    }
    // Only a chain that returns to its start encloses a region; a single line, arc or
    // curve cannot, since its ends are distinct.
    !closed || segments.last().expect("non-empty profile").end_mm() == segments[0].start_mm()
}

pub(super) fn is_valid_profile(points_mm: &[[f64; 2]]) -> bool {
    if !(3..=MAX_PROFILE_POINTS).contains(&points_mm.len())
        || points_mm
            .iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_COORDINATE_MM)
    {
        return false;
    }
    for (index, point) in points_mm.iter().enumerate() {
        if points_mm[index + 1..].iter().any(|candidate| {
            (point[0] - candidate[0]).abs() <= PROFILE_EPSILON_MM
                && (point[1] - candidate[1]).abs() <= PROFILE_EPSILON_MM
        }) {
            return false;
        }
    }
    let twice_area: f64 = points_mm
        .iter()
        .zip(points_mm.iter().cycle().skip(1))
        .take(points_mm.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum();
    // Either direction encloses the same region; drawn and imported loops use both.
    if twice_area.abs() <= PROFILE_EPSILON_MM {
        return false;
    }
    for left_index in 0..points_mm.len() {
        let left_next = (left_index + 1) % points_mm.len();
        for right_index in (left_index + 1)..points_mm.len() {
            let right_next = (right_index + 1) % points_mm.len();
            if left_index == right_next || left_next == right_index {
                continue;
            }
            if segments_intersect(
                points_mm[left_index],
                points_mm[left_next],
                points_mm[right_index],
                points_mm[right_next],
            ) {
                return false;
            }
        }
    }
    true
}

pub(super) fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    /// Positive when `c` lies left of the line from `a` to `b`.
    fn orientation(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
        ketchup_geometry::linalg::cross2([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]])
    }
    fn on_segment(a: [f64; 2], b: [f64; 2], point: [f64; 2]) -> bool {
        point[0] >= a[0].min(b[0]) - PROFILE_EPSILON_MM
            && point[0] <= a[0].max(b[0]) + PROFILE_EPSILON_MM
            && point[1] >= a[1].min(b[1]) - PROFILE_EPSILON_MM
            && point[1] <= a[1].max(b[1]) + PROFILE_EPSILON_MM
    }
    let ab_c = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);
    if ((ab_c > PROFILE_EPSILON_MM && ab_d < -PROFILE_EPSILON_MM)
        || (ab_c < -PROFILE_EPSILON_MM && ab_d > PROFILE_EPSILON_MM))
        && ((cd_a > PROFILE_EPSILON_MM && cd_b < -PROFILE_EPSILON_MM)
            || (cd_a < -PROFILE_EPSILON_MM && cd_b > PROFILE_EPSILON_MM))
    {
        return true;
    }
    (ab_c.abs() <= PROFILE_EPSILON_MM && on_segment(a, b, c))
        || (ab_d.abs() <= PROFILE_EPSILON_MM && on_segment(a, b, d))
        || (cd_a.abs() <= PROFILE_EPSILON_MM && on_segment(c, d, a))
        || (cd_b.abs() <= PROFILE_EPSILON_MM && on_segment(c, d, b))
}
