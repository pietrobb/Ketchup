//! Persistent support plane, independent of geometry and program ownership.
use super::*;

impl Snapshot {
    /// Explicit world-Z support height; absent means the conventional z=0 plane.
    #[must_use]
    pub fn floor_z_mm(&self) -> Option<f64> {
        self.product.floor_z_mm
    }

    pub fn grounded_instances(&self) -> &BTreeSet<InstancePath> {
        &self.product.grounded_instances
    }

    #[must_use]
    pub fn instance_is_grounded(&self, path: &InstancePath) -> bool {
        self.occurrence_is_grounded(path.root_occurrence())
            || self.product.grounded_instances.iter().any(|anchor| {
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
        CanonicalCommand::SetGroundedInstances { .. } => {
            dependencies.insert(AuthoritativeDependency::GroundedInstances);
        }
        CanonicalCommand::SetFloorHeight { .. } => {
            dependencies.insert(AuthoritativeDependency::FloorHeight);
        }
        CanonicalCommand::SetTolerance { .. } => {
            dependencies.insert(AuthoritativeDependency::Tolerance);
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
    product.grounded_instances = paths.clone();
    Ok(())
}

pub(super) fn validate_grounded_instances(
    product: &ProductModel,
    paths: &BTreeSet<InstancePath>,
) -> Result<(), CanonicalError> {
    if paths
        .iter()
        .any(|path| resolve_product_instance_path(product, path).is_none())
    {
        return Err(CanonicalError::InvalidInstancePath);
    }
    Ok(())
}

pub(super) fn prune_grounded_instances(product: &mut ProductModel) {
    product.grounded_instances = product
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
