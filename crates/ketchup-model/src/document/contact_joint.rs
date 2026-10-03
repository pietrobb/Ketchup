//! Physical contact declarations, not kinematic constraints.
use super::*;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContactJoint {
    pub name: String,
    pub parts: [InstancePath; 2],
    pub max_gap_mm: f64,
}

impl Snapshot {
    pub fn contact_joints(&self) -> &[ContactJoint] {
        &self.product.support.contact_joints
    }
}

pub(super) fn validate(
    product: &ProductModel,
    joints: &[ContactJoint],
) -> Result<(), CanonicalError> {
    for joint in joints {
        if !joint.max_gap_mm.is_finite() || joint.max_gap_mm < 0.0 {
            return Err(CanonicalError::DimensionOutsideEnvelope);
        }
        for path in &joint.parts {
            if resolve_product_instance_path(product, path).is_none() {
                return Err(CanonicalError::InvalidInstancePath);
            }
        }
    }
    Ok(())
}

pub(super) fn set(
    product: &mut ProductModel,
    joints: &[ContactJoint],
) -> Result<(), CanonicalError> {
    validate(product, joints)?;
    product.support.contact_joints = joints.to_vec();
    Ok(())
}

pub(super) fn prune(product: &mut ProductModel) {
    product.support.contact_joints = product
        .support
        .contact_joints
        .iter()
        .filter(|joint| {
            joint
                .parts
                .iter()
                .all(|path| resolve_product_instance_path(product, path).is_some())
        })
        .cloned()
        .collect();
}

pub(super) fn dependencies(
    snapshot: &Snapshot,
    joints: &[ContactJoint],
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    for joint in joints.iter().chain(snapshot.contact_joints()) {
        for path in &joint.parts {
            support::add_path_dependencies(snapshot, path, dependencies);
        }
    }
}
