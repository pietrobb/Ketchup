//! Joints, spaces, clearance volumes, CAM plans, pin joints, the assembly recipe, persistent dimensions and drawing sheets.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::UpsertJoint(joint) => joint::upsert(&mut edit.product, joint)?,
        CanonicalCommand::DeleteJoint { id } => joint::delete(&mut edit.product, *id)?,
        CanonicalCommand::UpsertSpace(space) => {
            edit.product
                .spaces
                .insert(space.id(), Arc::new(space.clone()));
        }
        CanonicalCommand::DeleteSpace { id } => {
            if edit.product.spaces.remove(id).is_none() {
                return Err(CanonicalError::SpaceNotFound(*id));
            }
        }
        CanonicalCommand::UpsertClearanceVolume(clearance) => {
            if clearance.derived_from().is_some_and(|identity| {
                resolve_derived_identity(&edit.product.evaluator_nodes, identity)
                    != SlotResolution::Resolved
            }) {
                return Err(CanonicalError::UnresolvedDerivedOutput);
            }
            edit.product
                .clearance_volumes
                .insert(clearance.id(), Arc::new(clearance.clone()));
        }
        CanonicalCommand::DeleteClearanceVolume { id } => {
            if edit.product.clearance_volumes.remove(id).is_none() {
                return Err(CanonicalError::ClearanceVolumeNotFound(*id));
            }
        }
        CanonicalCommand::UpsertCamPlan(plan) => {
            let candidate = Snapshot {
                revision_id: edit.current.revision_id(),
                product: Arc::new(edit.product.clone()),
            };
            plan.validate(&candidate).map_err(CanonicalError::Cam)?;
            edit.product
                .cam_plans
                .insert(plan.id(), Arc::new(plan.clone()));
        }
        CanonicalCommand::DeleteCamPlan { id } => {
            if edit.product.cam_plans.remove(id).is_none() {
                return Err(CanonicalError::CamPlanNotFound(*id));
            }
        }
        CanonicalCommand::UpsertPinJoint(joint) => {
            let candidate = Snapshot {
                revision_id: edit.current.revision_id(),
                product: Arc::new(edit.product.clone()),
            };
            project_pin_joint_contract(&candidate, joint).map_err(CanonicalError::PinJoint)?;
            edit.product
                .pin_joints
                .insert(joint.id, Arc::new(joint.clone()));
        }
        CanonicalCommand::DeletePinJoint { id } => {
            if edit.product.pin_joints.remove(id).is_none() {
                return Err(CanonicalError::PinJointNotFound(*id));
            }
        }
        CanonicalCommand::SetAssemblyRecipe(recipe) => {
            let candidate = Snapshot {
                revision_id: edit.current.revision_id(),
                product: Arc::new(edit.product.clone()),
            };
            recipe
                .audit(&candidate)
                .map_err(CanonicalError::AssemblyRecipe)?;
            edit.product.assembly_recipe = Some(Arc::new(recipe.clone()));
        }
        CanonicalCommand::ClearAssemblyRecipe => {
            if edit.product.assembly_recipe.take().is_none() {
                return Err(CanonicalError::AssemblyRecipeNotFound);
            }
        }
        CanonicalCommand::UpsertPersistentDimension(dimension) => {
            validate_persistent_dimension(dimension)?;
            edit.product
                .persistent_dimensions
                .insert(dimension.id, Arc::new(dimension.clone()));
        }
        CanonicalCommand::DeletePersistentDimension { id } => {
            if edit.product.persistent_dimensions.remove(id).is_none() {
                return Err(CanonicalError::PersistentDimensionNotFound(*id));
            }
        }
        CanonicalCommand::CreateDrawingSheet(sheet) => {
            if edit.product.drawing_sheets.contains_key(&sheet.id()) {
                return Err(CanonicalError::DrawingSheetAlreadyExists(sheet.id()));
            }
            validate_drawing_sheet(&validation_snapshot(&edit.product), sheet)?;
            edit.product
                .drawing_sheets
                .insert(sheet.id(), Arc::new(sheet.clone()));
        }
        CanonicalCommand::UpdateDrawingSheet(sheet) => {
            if !edit.product.drawing_sheets.contains_key(&sheet.id()) {
                return Err(CanonicalError::DrawingSheetNotFound(sheet.id()));
            }
            validate_drawing_sheet(&validation_snapshot(&edit.product), sheet)?;
            edit.product
                .drawing_sheets
                .insert(sheet.id(), Arc::new(sheet.clone()));
        }
        CanonicalCommand::DeleteDrawingSheet { id } => {
            edit.product
                .drawing_sheets
                .remove(id)
                .ok_or(CanonicalError::DrawingSheetNotFound(*id))?;
        }
        _ => return Err(CanonicalError::CommandOutsideFamily("annotation")),
    }
    Ok(())
}
