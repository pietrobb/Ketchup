use super::*;

pub(super) fn resolve_product_instance_path(
    product: &ProductModel,
    path: &InstancePath,
) -> Option<DefinitionId> {
    let root = product.occurrences.get(&path.root_occurrence())?;
    let mut definition_id = root.definition_id;
    let mut parent = None;
    for step in path.steps() {
        match *step {
            InstancePathStep::Group(local_id) => {
                let group = product.local_groups.get(&LocalGroupKey {
                    definition_id,
                    local_id,
                })?;
                if group.parent != parent {
                    return None;
                }
                parent = Some(local_id);
            }
            InstancePathStep::Occurrence(local_id) => {
                let occurrence = product.local_occurrences.get(&LocalOccurrenceKey {
                    definition_id,
                    local_id,
                })?;
                if occurrence.parent != parent {
                    return None;
                }
                definition_id = occurrence.definition_id;
                parent = None;
            }
        }
    }
    Some(definition_id)
}

pub(super) fn validate_assembly_mate(
    product: &ProductModel,
    mate: &AssemblyMate,
    require_resolved: bool,
) -> Result<(), CanonicalError> {
    if mate.schema() != ASSEMBLY_MATE_SCHEMA_V1
        || mate.id().0 == 0
        || !mate.kind().is_valid()
        || mate.endpoint_a().instance_path() == mate.endpoint_b().instance_path()
    {
        return Err(CanonicalError::InvalidAssemblyMate(mate.id()));
    }
    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
        let definition_id = resolve_product_instance_path(product, endpoint.instance_path())
            .ok_or(CanonicalError::InvalidInstancePath)?;
        let reference = endpoint.reference();
        let health_is_valid = match endpoint.health() {
            AssemblyReferenceHealth::Resolved => true,
            AssemblyReferenceHealth::Broken | AssemblyReferenceHealth::Lost => !require_resolved,
            AssemblyReferenceHealth::Ambiguous { candidate_count } => {
                !require_resolved && candidate_count > 1
            }
        };
        let attachment_is_valid = match endpoint.attachment() {
            AssemblyMateAttachment::ReferenceOnly(_) => true,
            AssemblyMateAttachment::PlanarFace(attachment) => attachment.has_valid_geometry(),
            AssemblyMateAttachment::Axial(attachment) => attachment.has_valid_geometry(),
        };
        if !health_is_valid
            || !attachment_is_valid
            || !reference.has_valid_lineage()
            || reference.document_id != product.document_id
            || reference.definition_id != definition_id
            || product
                .features
                .get(&reference.profile_feature_id)
                .is_none_or(|feature| feature.definition_id != definition_id)
            || product
                .features
                .get(&reference.producer_feature_id)
                .is_none_or(|feature| feature.definition_id != definition_id)
        {
            return Err(CanonicalError::InvalidAssemblyMate(mate.id()));
        }
    }
    let type_is_valid = match mate.kind() {
        AssemblyMateKind::CoincidentPlanar { .. }
        | AssemblyMateKind::Distance { .. }
        | AssemblyMateKind::Angle { .. } => {
            [mate.endpoint_a(), mate.endpoint_b()]
                .iter()
                .all(|endpoint| {
                    matches!(endpoint.attachment(), AssemblyMateAttachment::PlanarFace(_))
                        || (!require_resolved
                            && matches!(
                                endpoint.attachment(),
                                AssemblyMateAttachment::ReferenceOnly(_)
                            ))
                })
        }
        AssemblyMateKind::ConcentricAxial { .. } => [mate.endpoint_a(), mate.endpoint_b()]
            .iter()
            .all(|endpoint| {
                matches!(endpoint.attachment(), AssemblyMateAttachment::Axial(_))
                    || (!require_resolved
                        && matches!(
                            endpoint.attachment(),
                            AssemblyMateAttachment::ReferenceOnly(_)
                        ))
            }),
    };
    if !type_is_valid {
        return Err(CanonicalError::InvalidAssemblyMate(mate.id()));
    }
    Ok(())
}

pub(super) fn validate_assembly_joint(
    product: &ProductModel,
    joint: &AssemblyJoint,
) -> Result<(), CanonicalError> {
    if !joint.has_valid_shape()
        || joint.schema() != ASSEMBLY_JOINT_SCHEMA_V1
        || resolve_product_instance_path(product, joint.parent_instance_path()).is_none()
        || resolve_product_instance_path(product, joint.child_instance_path()).is_none()
        || product.assembly_joints.values().any(|existing| {
            existing.id() != joint.id()
                && existing.child_instance_path() == joint.child_instance_path()
        })
    {
        return Err(CanonicalError::InvalidAssemblyJoint(joint.id()));
    }

    let mut parent_by_child = product
        .assembly_joints
        .values()
        .filter(|existing| existing.id() != joint.id())
        .map(|existing| {
            (
                existing.child_instance_path().clone(),
                existing.parent_instance_path().clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    parent_by_child.insert(
        joint.child_instance_path().clone(),
        joint.parent_instance_path().clone(),
    );
    let mut cursor = joint.parent_instance_path().clone();
    let mut visited = BTreeSet::new();
    while let Some(parent) = parent_by_child.get(&cursor).cloned() {
        if parent == *joint.child_instance_path() || !visited.insert(cursor) {
            return Err(CanonicalError::InvalidAssemblyJoint(joint.id()));
        }
        cursor = parent;
    }
    Ok(())
}

pub(super) fn validate_assembly_motion_coupling(
    product: &ProductModel,
    coupling: &AssemblyMotionCoupling,
) -> Result<(), CanonicalError> {
    if !coupling.has_valid_shape() || coupling.schema() != ASSEMBLY_MOTION_COUPLING_SCHEMA_V1 {
        return Err(CanonicalError::InvalidAssemblyMotionCoupling(coupling.id()));
    }
    let input = product
        .assembly_joints
        .get(&coupling.input_joint_id())
        .ok_or(CanonicalError::AssemblyJointNotFound(
            coupling.input_joint_id(),
        ))?;
    let output = product
        .assembly_joints
        .get(&coupling.output_joint_id())
        .ok_or(CanonicalError::AssemblyJointNotFound(
            coupling.output_joint_id(),
        ))?;
    let (expected_input, expected_output) = coupling.transmission().joint_kinds();
    if joint_kind_class(input.kind()) != Some(expected_input)
        || joint_kind_class(output.kind()) != Some(expected_output)
        || input
            .kind()
            .with_position(coupling.input_reference_position())
            .is_none_or(|kind| !kind.is_valid())
        || output
            .kind()
            .with_position(coupling.output_reference_position())
            .is_none_or(|kind| !kind.is_valid())
        || !motion_values_equal(
            output
                .kind()
                .position()
                .expect("typed coupling output is movable"),
            coupling.output_position(
                input
                    .kind()
                    .position()
                    .expect("typed coupling input is movable"),
            ),
        )
    {
        return Err(CanonicalError::InvalidAssemblyMotionCoupling(coupling.id()));
    }

    let mut couplings = product
        .assembly_motion_couplings
        .values()
        .filter(|existing| existing.id() != coupling.id())
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    couplings.push(coupling);
    validate_motion_coupling_graph(&couplings)
        .map_err(CanonicalError::InvalidAssemblyMotionCoupling)
}

pub(super) fn validate_mechanical_interface(
    product: &ProductModel,
    interface: &MechanicalInterface,
) -> Result<(), CanonicalError> {
    if !interface.has_valid_shape() || interface.schema() != MECHANICAL_INTERFACE_SCHEMA_V1 {
        return Err(CanonicalError::InvalidMechanicalInterface(interface.id()));
    }
    if !product.occurrences.contains_key(&interface.occurrence_id()) {
        return Err(CanonicalError::OccurrenceNotFound(
            interface.occurrence_id(),
        ));
    }
    Ok(())
}

pub(super) fn validate_mechanical_condition(
    product: &ProductModel,
    condition: &MechanicalCondition,
) -> Result<(), CanonicalError> {
    if !condition.has_valid_shape() || condition.schema() != MECHANICAL_CONDITION_SCHEMA_V1 {
        return Err(CanonicalError::InvalidMechanicalCondition(condition.id()));
    }
    for interface_id in condition.kind().interfaces() {
        if !product.mechanical_interfaces.contains_key(&interface_id) {
            return Err(CanonicalError::MechanicalInterfaceNotFound(interface_id));
        }
    }
    if let Some(joint_id) = condition.kind().joint_id()
        && !product.assembly_joints.contains_key(&joint_id)
    {
        return Err(CanonicalError::AssemblyJointNotFound(joint_id));
    }
    if let MechanicalConditionKind::JointAxisAlignment { joint_id, .. } = condition.kind()
        && product
            .assembly_joints
            .get(&joint_id)
            .is_none_or(|joint| joint.kind().axis().is_none())
    {
        return Err(CanonicalError::InvalidMechanicalCondition(condition.id()));
    }
    Ok(())
}

pub(super) fn joint_kind_class(kind: AssemblyJointKind) -> Option<CoupledJointKind> {
    match kind {
        AssemblyJointKind::Fixed => None,
        AssemblyJointKind::Revolute { .. } | AssemblyJointKind::Helical { .. } => {
            Some(CoupledJointKind::Revolute)
        }
        AssemblyJointKind::Prismatic { .. } => Some(CoupledJointKind::Prismatic),
    }
}

pub(super) fn validate_motion_coupling_graph(
    couplings: &[&AssemblyMotionCoupling],
) -> Result<(), AssemblyMotionCouplingId> {
    let mut adjacency = BTreeMap::<
        AssemblyJointId,
        Vec<(AssemblyJointId, f64, f64, AssemblyMotionCouplingId)>,
    >::new();
    for coupling in couplings {
        let scale = coupling.transmission().scale();
        let offset =
            coupling.output_reference_position() - scale * coupling.input_reference_position();
        adjacency
            .entry(coupling.input_joint_id())
            .or_default()
            .push((coupling.output_joint_id(), scale, offset, coupling.id()));
        adjacency
            .entry(coupling.output_joint_id())
            .or_default()
            .push((
                coupling.input_joint_id(),
                scale.recip(),
                -offset / scale,
                coupling.id(),
            ));
    }

    let mut unresolved = adjacency.keys().copied().collect::<BTreeSet<_>>();
    while let Some(root) = unresolved.pop_first() {
        let mut transforms = BTreeMap::from([(root, (1.0_f64, 0.0_f64))]);
        let mut pending = vec![root];
        while let Some(joint_id) = pending.pop() {
            let (source_scale, source_offset) = transforms[&joint_id];
            for (neighbour, edge_scale, edge_offset, coupling_id) in
                adjacency.get(&joint_id).into_iter().flatten()
            {
                let candidate = (
                    edge_scale * source_scale,
                    edge_scale * source_offset + edge_offset,
                );
                if !candidate.0.is_finite()
                    || candidate.0 == 0.0
                    || !candidate.0.recip().is_finite()
                    || !candidate.1.is_finite()
                    || !(-candidate.1 / candidate.0).is_finite()
                {
                    return Err(*coupling_id);
                }
                if let Some(existing) = transforms.get(neighbour) {
                    if !motion_values_equal(existing.0, candidate.0)
                        || !motion_values_equal(existing.1, candidate.1)
                    {
                        return Err(*coupling_id);
                    }
                } else {
                    transforms.insert(*neighbour, candidate);
                    unresolved.remove(neighbour);
                    pending.push(*neighbour);
                }
            }
        }
    }
    Ok(())
}

pub(super) fn motion_values_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= ROUNDING * scale
}

pub(super) fn validate_assembly_motion_study(
    product: &ProductModel,
    study: &AssemblyMotionStudy,
) -> Result<(), CanonicalError> {
    if !study.has_valid_shape() || study.schema() != ASSEMBLY_MOTION_STUDY_SCHEMA_V1 {
        return Err(CanonicalError::InvalidAssemblyMotionStudy(study.id()));
    }
    ensure_name(study.name())?;
    for driver in study.drivers() {
        let joint = product
            .assembly_joints
            .get(&driver.joint_id())
            .ok_or(CanonicalError::AssemblyJointNotFound(driver.joint_id()))?;
        if joint
            .kind()
            .with_position(driver.position())
            .is_none_or(|kind| !kind.is_valid())
        {
            return Err(CanonicalError::InvalidAssemblyMotionStudy(study.id()));
        }
    }
    Ok(())
}

pub(super) fn validate_drawing_sheet(
    product: &ProductModel,
    sheet: &DrawingSheet,
) -> Result<(), CanonicalError> {
    ensure_product_id(sheet.id().0)?;
    ensure_name(sheet.name())?;
    if sheet.schema() != crate::drawing::ORTHOGRAPHIC_DRAWING_SCHEMA_V2 {
        return Err(CanonicalError::Drawing(DrawingError::InvalidSheet));
    }
    let snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    crate::drawing::validate_source(&snapshot, sheet.source()).map_err(CanonicalError::Drawing)
}

pub(super) fn validate_assembly_joint_motion_publication(
    current: &Snapshot,
    product: &ProductModel,
    batch: &CommandBatch,
) -> Result<(), CanonicalError> {
    let final_joints_by_child = product
        .assembly_joints
        .values()
        .map(|joint| (joint.child_occurrence_id(), joint.as_ref()))
        .collect::<BTreeMap<_, _>>();
    for before in current.assembly_joints() {
        if let Some(after) = product.assembly_joints.get(&before.id())
            && (before.parent_occurrence_id() != after.parent_occurrence_id()
                || before.child_occurrence_id() != after.child_occurrence_id())
        {
            return Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
                before.id(),
            ));
        }
        if let Some(after) = final_joints_by_child.get(&before.child_occurrence_id())
            && after.id() != before.id()
        {
            return Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
                before.id(),
            ));
        }
    }

    let kind_overrides = current
        .assembly_joints()
        .filter_map(|before| {
            product
                .assembly_joints
                .get(&before.id())
                .filter(|after| !joint_motion_states_equal(before.kind(), after.kind()))
                .map(|after| (before.id(), after.kind()))
        })
        .collect::<BTreeMap<_, _>>();
    let Some(first_changed_joint_id) = kind_overrides.keys().next().copied() else {
        return Ok(());
    };

    let expected_solution =
        solve_assembly_joint_kinematics_with_kind_overrides(current, &kind_overrides)
            .map_err(|error| CanonicalError::UnsolvedAssemblySolvePublication(Box::new(error)))?;
    let required_transform_ids = kind_overrides
        .keys()
        .filter_map(|id| {
            current.assembly_joint(*id).and_then(|joint| {
                joint
                    .child_instance_path()
                    .is_root()
                    .then_some(joint.child_occurrence_id())
            })
        })
        .collect::<BTreeSet<_>>();
    let expected_transforms = expected_solution
        .poses()
        .iter()
        .filter(|pose| pose.instance_path().is_root())
        .filter_map(|pose| {
            current
                .occurrence(pose.occurrence_id())
                .filter(|occurrence| {
                    required_transform_ids.contains(&pose.occurrence_id())
                        || !transforms_equivalent(occurrence.transform(), pose.local_transform())
                })
                .map(|_| (pose.occurrence_id(), pose.local_transform()))
        })
        .collect::<Vec<_>>();
    let expected_instance_transforms = expected_solution
        .poses()
        .iter()
        .filter(|pose| !pose.instance_path().is_root())
        .filter_map(|pose| {
            current
                .resolve_instance_path(pose.instance_path())
                .ok()
                .filter(|resolved| {
                    !transforms_equivalent(resolved.local_transform, pose.local_transform())
                })
                .map(|_| (pose.instance_path().clone(), pose.local_transform()))
        })
        .collect::<Vec<_>>();
    let solve_publications = batch
        .commands
        .iter()
        .filter_map(|command| match command {
            CanonicalCommand::ApplyAssemblySolve {
                transforms,
                instance_transforms,
                ..
            } => Some((transforms, instance_transforms)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if solve_publications.len() != 1
        || solve_publications[0].0.len() != expected_transforms.len()
        || solve_publications[0]
            .0
            .iter()
            .zip(&expected_transforms)
            .any(|((actual_id, actual), (expected_id, expected))| {
                actual_id != expected_id || !transforms_equivalent(*actual, *expected)
            })
        || solve_publications[0].1.len() != expected_instance_transforms.len()
        || solve_publications[0]
            .1
            .iter()
            .zip(&expected_instance_transforms)
            .any(|((actual_path, actual), (expected_path, expected))| {
                actual_path != expected_path || !transforms_equivalent(*actual, *expected)
            })
        || expected_transforms.iter().any(|(id, expected)| {
            product
                .occurrences
                .get(id)
                .is_none_or(|occurrence| !transforms_equivalent(occurrence.transform(), *expected))
        })
        || expected_instance_transforms.iter().any(|(path, expected)| {
            product
                .instance_transform_overrides
                .get(path)
                .is_none_or(|actual| !transforms_equivalent(*actual, *expected))
        })
    {
        return Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
            first_changed_joint_id,
        ));
    }
    Ok(())
}

pub(crate) fn validate_production_code(code: &str) -> Result<(), CanonicalError> {
    if code.is_empty()
        || code.len() > 64
        || !code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
    {
        return Err(CanonicalError::InvalidProductionCode);
    }
    Ok(())
}

// Include every owning definition, not merely the leaf: reused local IDs must not rebind codes.
pub(super) fn production_code_identity(
    product: &ProductModel,
    path: &InstancePath,
) -> Option<Vec<DefinitionId>> {
    if path.steps().len() > 256 || matches!(path.steps().last(), Some(InstancePathStep::Group(_))) {
        return None;
    }
    let mut prefix = InstancePath::root(path.root_occurrence());
    let mut identity = vec![resolve_product_instance_path(product, &prefix)?];
    for step in path.steps() {
        prefix = prefix.with_step(*step);
        identity.push(resolve_product_instance_path(product, &prefix)?);
    }
    let definition = product.definitions.get(identity.last()?)?;
    if !definition.feature_ids.iter().any(|id| {
        product
            .features
            .get(id)
            .is_some_and(|feature| feature.kind.produces_body())
    }) {
        return None;
    }
    Some(identity)
}
