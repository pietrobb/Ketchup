//! Assembly solves, mates, joints, motion couplings and studies, and mechanical interfaces and conditions.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::GuardAssemblyRecompute {
            source_revision,
            source_digest,
        } => {
            if edit.current.revision_id() != *source_revision
                || edit.current.canonical_digest() != *source_digest
            {
                return Err(CanonicalError::StaleAssemblySolve);
            }
        }
        CanonicalCommand::ApplyAssemblySolve {
            source_revision,
            source_digest,
            transforms,
            instance_transforms,
        } => {
            if edit.current.revision_id() != *source_revision
                || edit.current.canonical_digest() != *source_digest
            {
                return Err(CanonicalError::StaleAssemblySolve);
            }
            if (transforms.is_empty() && instance_transforms.is_empty())
                || transforms.windows(2).any(|pair| pair[0].0 >= pair[1].0)
                || instance_transforms
                    .windows(2)
                    .any(|pair| pair[0].0 >= pair[1].0)
            {
                return Err(CanonicalError::InvalidAssemblySolvePublication);
            }
            for (id, transform) in transforms {
                validate_transform(*transform)?;
                if edit.product.grounded_occurrences.contains(id) {
                    return Err(CanonicalError::InvalidAssemblySolvePublication);
                }
                let existing = edit
                    .product
                    .occurrences
                    .get(id)
                    .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                edit.product.occurrences.insert(
                    *id,
                    Arc::new(Occurrence {
                        transform: *transform,
                        ..existing.as_ref().clone()
                    }),
                );
            }
            for (path, transform) in instance_transforms {
                validate_transform(*transform)?;
                if path.is_root()
                    || !matches!(path.steps().last(), Some(InstancePathStep::Occurrence(_)))
                    || resolve_product_instance_path(&edit.product, path).is_none()
                {
                    return Err(CanonicalError::InvalidInstancePath);
                }
                edit.product
                    .instance_transform_overrides
                    .insert(path.clone(), *transform);
            }
        }
        CanonicalCommand::CreateAssemblyMate(mate) => {
            ensure_product_id(mate.id().0)?;
            if edit.product.assembly_mates.contains_key(&mate.id()) {
                return Err(CanonicalError::AssemblyMateAlreadyExists(mate.id()));
            }
            validate_assembly_mate(&edit.product, mate, true)?;
            edit.product
                .assembly_mates
                .insert(mate.id(), Arc::new(mate.clone()));
        }
        CanonicalCommand::RebindAssemblyMate(mate) => {
            let existing = edit
                .product
                .assembly_mates
                .get(&mate.id())
                .ok_or(CanonicalError::AssemblyMateNotFound(mate.id()))?;
            if existing.kind() != mate.kind() {
                return Err(CanonicalError::InvalidAssemblyMate(mate.id()));
            }
            validate_assembly_mate(&edit.product, mate, false)?;
            edit.product
                .assembly_mates
                .insert(mate.id(), Arc::new(mate.clone()));
        }
        CanonicalCommand::SetAssemblyMateKind { id, kind } => {
            if !kind.is_valid() {
                return Err(CanonicalError::InvalidAssemblyMate(*id));
            }
            let existing = edit
                .product
                .assembly_mates
                .get(id)
                .ok_or(CanonicalError::AssemblyMateNotFound(*id))?;
            let replacement = AssemblyMate {
                kind: *kind,
                ..existing.as_ref().clone()
            };
            validate_assembly_mate(&edit.product, &replacement, true)?;
            edit.product
                .assembly_mates
                .insert(*id, Arc::new(replacement));
        }
        CanonicalCommand::DeleteAssemblyMate { id } => {
            edit.product
                .assembly_mates
                .remove(id)
                .ok_or(CanonicalError::AssemblyMateNotFound(*id))?;
        }
        CanonicalCommand::CreateAssemblyJoint(joint) => {
            ensure_product_id(joint.id().0)?;
            if edit.product.assembly_joints.contains_key(&joint.id()) {
                return Err(CanonicalError::AssemblyJointAlreadyExists(joint.id()));
            }
            validate_assembly_joint(&edit.product, joint)?;
            edit.product
                .assembly_joints
                .insert(joint.id(), Arc::new(joint.clone()));
        }
        CanonicalCommand::SetAssemblyJointKind { id, kind } => {
            let existing = edit
                .product
                .assembly_joints
                .get(id)
                .ok_or(CanonicalError::AssemblyJointNotFound(*id))?;
            let replacement = AssemblyJoint {
                kind: *kind,
                ..existing.as_ref().clone()
            };
            validate_assembly_joint(&edit.product, &replacement)?;
            edit.product
                .assembly_joints
                .insert(*id, Arc::new(replacement));
        }
        CanonicalCommand::SetAssemblyJointPosition { id, position } => {
            let existing = edit
                .product
                .assembly_joints
                .get(id)
                .ok_or(CanonicalError::AssemblyJointNotFound(*id))?;
            let kind = existing
                .kind()
                .with_position(*position)
                .ok_or(CanonicalError::InvalidAssemblyJoint(*id))?;
            let replacement = AssemblyJoint {
                kind,
                ..existing.as_ref().clone()
            };
            validate_assembly_joint(&edit.product, &replacement)?;
            edit.product
                .assembly_joints
                .insert(*id, Arc::new(replacement));
        }
        CanonicalCommand::SetAssemblyJointLimits { id, limits } => {
            let existing = edit
                .product
                .assembly_joints
                .get(id)
                .ok_or(CanonicalError::AssemblyJointNotFound(*id))?;
            let kind = existing
                .kind()
                .with_limits(*limits)
                .ok_or(CanonicalError::InvalidAssemblyJoint(*id))?;
            let replacement = AssemblyJoint {
                kind,
                ..existing.as_ref().clone()
            };
            validate_assembly_joint(&edit.product, &replacement)?;
            edit.product
                .assembly_joints
                .insert(*id, Arc::new(replacement));
        }
        CanonicalCommand::DeleteAssemblyJoint { id } => {
            if edit.product.assembly_motion_studies.values().any(|study| {
                study
                    .drivers()
                    .iter()
                    .any(|driver| driver.joint_id() == *id)
            }) {
                return Err(CanonicalError::AssemblyJointInMotionStudy(*id));
            }
            if edit
                .product
                .assembly_motion_couplings
                .values()
                .any(|coupling| {
                    coupling.input_joint_id() == *id || coupling.output_joint_id() == *id
                })
            {
                return Err(CanonicalError::AssemblyJointInMotionCoupling(*id));
            }
            edit.product
                .assembly_joints
                .remove(id)
                .ok_or(CanonicalError::AssemblyJointNotFound(*id))?;
        }
        CanonicalCommand::CreateAssemblyMotionCoupling(coupling) => {
            ensure_product_id(coupling.id().0)?;
            if edit
                .product
                .assembly_motion_couplings
                .contains_key(&coupling.id())
            {
                return Err(CanonicalError::AssemblyMotionCouplingAlreadyExists(
                    coupling.id(),
                ));
            }
            validate_assembly_motion_coupling(&edit.product, coupling)?;
            edit.product
                .assembly_motion_couplings
                .insert(coupling.id(), Arc::new(coupling.clone()));
        }
        CanonicalCommand::UpdateAssemblyMotionCoupling(coupling) => {
            if !edit
                .product
                .assembly_motion_couplings
                .contains_key(&coupling.id())
            {
                return Err(CanonicalError::AssemblyMotionCouplingNotFound(
                    coupling.id(),
                ));
            }
            validate_assembly_motion_coupling(&edit.product, coupling)?;
            edit.product
                .assembly_motion_couplings
                .insert(coupling.id(), Arc::new(coupling.clone()));
        }
        CanonicalCommand::DeleteAssemblyMotionCoupling { id } => {
            edit.product
                .assembly_motion_couplings
                .remove(id)
                .ok_or(CanonicalError::AssemblyMotionCouplingNotFound(*id))?;
        }
        CanonicalCommand::CreateAssemblyMotionStudy(study) => {
            ensure_product_id(study.id().0)?;
            if edit
                .product
                .assembly_motion_studies
                .contains_key(&study.id())
            {
                return Err(CanonicalError::AssemblyMotionStudyAlreadyExists(study.id()));
            }
            validate_assembly_motion_study(&edit.product, study)?;
            edit.product
                .assembly_motion_studies
                .insert(study.id(), Arc::new(study.clone()));
        }
        CanonicalCommand::UpdateAssemblyMotionStudy(study) => {
            if !edit
                .product
                .assembly_motion_studies
                .contains_key(&study.id())
            {
                return Err(CanonicalError::AssemblyMotionStudyNotFound(study.id()));
            }
            validate_assembly_motion_study(&edit.product, study)?;
            edit.product
                .assembly_motion_studies
                .insert(study.id(), Arc::new(study.clone()));
        }
        CanonicalCommand::DeleteAssemblyMotionStudy { id } => {
            edit.product
                .assembly_motion_studies
                .remove(id)
                .ok_or(CanonicalError::AssemblyMotionStudyNotFound(*id))?;
        }
        CanonicalCommand::CreateMechanicalInterface(interface) => {
            ensure_product_id(interface.id().0)?;
            if edit
                .product
                .mechanical_interfaces
                .contains_key(&interface.id())
            {
                return Err(CanonicalError::MechanicalInterfaceAlreadyExists(
                    interface.id(),
                ));
            }
            validate_mechanical_interface(&edit.product, interface)?;
            edit.product
                .mechanical_interfaces
                .insert(interface.id(), Arc::new(interface.clone()));
        }
        CanonicalCommand::UpdateMechanicalInterface(interface) => {
            if !edit
                .product
                .mechanical_interfaces
                .contains_key(&interface.id())
            {
                return Err(CanonicalError::MechanicalInterfaceNotFound(interface.id()));
            }
            validate_mechanical_interface(&edit.product, interface)?;
            edit.product
                .mechanical_interfaces
                .insert(interface.id(), Arc::new(interface.clone()));
        }
        CanonicalCommand::DeleteMechanicalInterface { id } => {
            if edit
                .product
                .mechanical_conditions
                .values()
                .any(|condition| condition.kind().interfaces().contains(id))
            {
                return Err(CanonicalError::MechanicalInterfaceInCondition(*id));
            }
            edit.product
                .mechanical_interfaces
                .remove(id)
                .ok_or(CanonicalError::MechanicalInterfaceNotFound(*id))?;
        }
        CanonicalCommand::CreateMechanicalCondition(condition) => {
            ensure_product_id(condition.id().0)?;
            if edit
                .product
                .mechanical_conditions
                .contains_key(&condition.id())
            {
                return Err(CanonicalError::MechanicalConditionAlreadyExists(
                    condition.id(),
                ));
            }
            validate_mechanical_condition(&edit.product, condition)?;
            edit.product
                .mechanical_conditions
                .insert(condition.id(), Arc::new(condition.clone()));
        }
        CanonicalCommand::UpdateMechanicalCondition(condition) => {
            if !edit
                .product
                .mechanical_conditions
                .contains_key(&condition.id())
            {
                return Err(CanonicalError::MechanicalConditionNotFound(condition.id()));
            }
            validate_mechanical_condition(&edit.product, condition)?;
            edit.product
                .mechanical_conditions
                .insert(condition.id(), Arc::new(condition.clone()));
        }
        CanonicalCommand::DeleteMechanicalCondition { id } => {
            edit.product
                .mechanical_conditions
                .remove(id)
                .ok_or(CanonicalError::MechanicalConditionNotFound(*id))?;
        }
        _ => {}
    }
    Ok(())
}
