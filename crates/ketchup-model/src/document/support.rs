//! Persistent support plane, independent of geometry and program ownership.
use super::*;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct SupportDeclarations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_z_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub grounded_instances: BTreeSet<InstancePath>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contact_joints: Vec<ContactJoint>,
}

impl Snapshot {
    /// Explicit world-Z support height; absent means the conventional z=0 plane.
    #[must_use]
    pub fn floor_z_mm(&self) -> Option<f64> {
        self.product.support.floor_z_mm
    }

    pub fn grounded_instances(&self) -> &BTreeSet<InstancePath> {
        &self.product.support.grounded_instances
    }

    #[must_use]
    pub fn instance_is_grounded(&self, path: &InstancePath) -> bool {
        self.occurrence_is_grounded(path.root_occurrence())
            || self
                .product
                .support
                .grounded_instances
                .iter()
                .any(|anchor| {
                    anchor.root_occurrence() == path.root_occurrence()
                        && path.steps().starts_with(anchor.steps())
                })
    }
}

pub(super) fn add_setting_dependency(
    command: &CanonicalCommand,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    match command {
        CanonicalCommand::SetContactJoints { .. } => {
            dependencies.insert(AuthoritativeDependency::ContactJoints);
        }
        CanonicalCommand::SetGroundedInstances { .. } => {
            dependencies.insert(AuthoritativeDependency::GroundedInstances);
        }
        CanonicalCommand::SetFloorHeight { .. } => {
            dependencies.insert(AuthoritativeDependency::FloorHeight);
        }
        CanonicalCommand::SetTolerance { .. } => {
            dependencies.insert(AuthoritativeDependency::Tolerance);
        }
        CanonicalCommand::SetProductionCode { .. } => {
            dependencies.insert(AuthoritativeDependency::ProductionCodes);
        }
        _ => {}
    }
}

pub(super) fn add_support_dependencies(
    snapshot: &Snapshot,
    command: &CanonicalCommand,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    add_setting_dependency(command, dependencies);
    if let CanonicalCommand::SetProductionCode { instance_path, .. } = command {
        add_path_dependencies(snapshot, instance_path, dependencies);
    }
    if let CanonicalCommand::SetContactJoints { joints } = command {
        contact_joint::dependencies(snapshot, joints, dependencies);
    }
    if let CanonicalCommand::SetGroundedInstances { paths } = command {
        for path in paths.iter().chain(snapshot.grounded_instances()) {
            add_path_dependencies(snapshot, path, dependencies);
        }
    }
}

pub(super) fn add_grounding_dependencies(
    id: OccurrenceId,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    dependencies.insert(AuthoritativeDependency::Occurrence(id));
    dependencies.insert(AuthoritativeDependency::GroundedOccurrence(id));
}

pub(super) fn add_path_dependencies(
    snapshot: &Snapshot,
    path: &InstancePath,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    dependencies.insert(AuthoritativeDependency::Occurrence(path.root_occurrence()));
    if let Some(owners) = production_code_identity(&snapshot.product, path) {
        dependencies.extend(owners.into_iter().map(AuthoritativeDependency::Definition));
    }
}

pub(super) fn set_grounded_instances(
    product: &mut ProductModel,
    paths: &BTreeSet<InstancePath>,
) -> Result<(), CanonicalError> {
    validate_grounded_instances(product, paths)?;
    product.support.grounded_instances = paths.clone();
    Ok(())
}

pub(super) fn validate_grounded_instances(
    product: &ProductModel,
    paths: &BTreeSet<InstancePath>,
) -> Result<(), CanonicalError> {
    match paths
        .iter()
        .find(|path| resolve_product_instance_path(product, path).is_none())
    {
        Some(path) => Err(CanonicalError::GroundedPathNotFound(path.clone())),
        None => Ok(()),
    }
}

pub(super) fn apply_setting(
    product: &mut ProductModel,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::SetFloorHeight { z_mm } => product.support.floor_z_mm = *z_mm,
        CanonicalCommand::SetContactJoints { joints } => contact_joint::set(product, joints)?,
        CanonicalCommand::SetOccurrenceGrounded { id, grounded } => {
            set_occurrence_grounded(product, *id, *grounded)?
        }
        CanonicalCommand::SetGroundedInstances { paths } => set_grounded_instances(product, paths)?,
        CanonicalCommand::SetTolerance { tolerance } => product.tolerance = *tolerance,
        _ => {}
    }
    Ok(())
}

pub(super) fn prune_grounded_instances(product: &mut ProductModel) {
    product.support.grounded_instances = product
        .support
        .grounded_instances
        .iter()
        .filter(|path| resolve_product_instance_path(product, path).is_some())
        .cloned()
        .collect();
}

pub(super) fn set_occurrence_grounded(
    product: &mut ProductModel,
    id: OccurrenceId,
    grounded: bool,
) -> Result<(), CanonicalError> {
    if !product.occurrences.contains_key(&id) {
        return Err(CanonicalError::OccurrenceNotFound(id));
    }
    if grounded {
        product.grounded_occurrences.insert(id);
    } else {
        product.grounded_occurrences.remove(&id);
    }
    Ok(())
}

pub(super) fn validate_floor(z_mm: Option<f64>) -> Result<(), CanonicalError> {
    if z_mm.is_some_and(|z| !z.is_finite() || z.abs() > MAX_COORDINATE_MM) {
        return Err(CanonicalError::DimensionOutsideEnvelope);
    }
    Ok(())
}
