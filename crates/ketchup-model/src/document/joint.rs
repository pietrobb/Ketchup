//! Declared joints of the document: two participants that may overlap inside a
//! declared volume. A participant is a value a rule derives or a placed part (an
//! ordinary or a program part), named by its instance path.
use super::*;

#[derive(
    Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum JointParticipant {
    Derived(DerivedIdentity),
    Part(InstancePath),
}

impl From<DerivedIdentity> for JointParticipant {
    fn from(identity: DerivedIdentity) -> Self {
        Self::Derived(identity)
    }
}

impl From<InstancePath> for JointParticipant {
    fn from(path: InstancePath) -> Self {
        Self::Part(path)
    }
}

impl JointParticipant {
    #[must_use]
    pub const fn part(&self) -> Option<&InstancePath> {
        match self {
            Self::Part(path) => Some(path),
            Self::Derived(_) => None,
        }
    }
}

/// A joint as the document stores it.
pub type DocumentJoint = CanonicalJoint<JointParticipant>;

fn parts(joint: &DocumentJoint) -> impl Iterator<Item = &InstancePath> {
    [joint.participant_a(), joint.participant_b()]
        .into_iter()
        .filter_map(JointParticipant::part)
}

/// Refuses a joint that names a part which is not placed in `product`.
pub(super) fn validate(
    product: &ProductModel,
    joint: &DocumentJoint,
) -> Result<(), CanonicalError> {
    match parts(joint).find(|path| resolve_product_instance_path(product, path).is_none()) {
        Some(path) => Err(CanonicalError::JointPartNotFound {
            joint: joint.id(),
            path: path.clone(),
        }),
        None => Ok(()),
    }
}

pub(super) fn upsert(
    product: &mut ProductModel,
    joint: &DocumentJoint,
) -> Result<(), CanonicalError> {
    validate(product, joint)?;
    product.joints.insert(joint.id(), Arc::new(joint.clone()));
    Ok(())
}

pub(super) fn delete(product: &mut ProductModel, id: JointId) -> Result<(), CanonicalError> {
    product
        .joints
        .remove(&id)
        .map(drop)
        .ok_or(CanonicalError::JointNotFound(id))
}

/// Drops the joints of parts that are no longer placed, like contact joints.
pub(super) fn prune(product: &mut ProductModel) {
    let removed = product
        .joints
        .values()
        .filter(|joint| {
            parts(joint).any(|path| resolve_product_instance_path(product, path).is_none())
        })
        .map(|joint| joint.id())
        .collect::<Vec<_>>();
    for id in removed {
        product.joints.remove(&id);
    }
}

pub(super) fn add_dependencies(
    snapshot: &Snapshot,
    joint: &DocumentJoint,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    for participant in [joint.participant_a(), joint.participant_b()] {
        match participant {
            JointParticipant::Derived(identity) => {
                add_evaluator_dependency_closure(
                    snapshot,
                    identity.root_rule_node_id,
                    dependencies,
                );
            }
            JointParticipant::Part(path) => {
                support::add_path_dependencies(snapshot, path, dependencies);
            }
        }
    }
}
