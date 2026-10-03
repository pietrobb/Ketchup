//! Explicit distance requests must not be discarded by collision broad-phase bounds.
use super::{Body, CollisionScope, contact_candidates};
use crate::validation::AssistantValidationSelection;
use ketchup_model::document::{InstancePath, Snapshot};
use std::collections::{BTreeMap, BTreeSet};

impl CollisionScope {
    /// Force native measurements of these instance pairs even when their bounds
    /// prove that they cannot collide. Both instances must belong to the snapshot.
    pub fn measuring_pairs(
        mut self,
        pairs: impl IntoIterator<Item = (InstancePath, InstancePath)>,
    ) -> Self {
        self.measured_pairs = pairs
            .into_iter()
            .map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
            .collect();
        self
    }
}

pub(super) fn required_pairs(
    snapshot: &Snapshot,
    selection: &AssistantValidationSelection,
    bodies: &[Body],
    scoped: &BTreeSet<usize>,
    scope: Option<&CollisionScope>,
) -> BTreeSet<(usize, usize)> {
    let mut pairs = contact_candidates(snapshot, selection, bodies, scoped);
    if let Some(scope) = scope {
        let mut indices: BTreeMap<&InstancePath, Vec<usize>> = BTreeMap::new();
        for (index, body) in bodies.iter().enumerate() {
            indices
                .entry(&body.occurrence.instance_path)
                .or_default()
                .push(index);
        }
        for (a, b) in &scope.measured_pairs {
            if let (Some(left), Some(right)) = (indices.get(a), indices.get(b)) {
                for &a in left {
                    for &b in right {
                        if a != b && (scoped.contains(&a) || scoped.contains(&b)) {
                            pairs.insert((a.min(b), a.max(b)));
                        }
                    }
                }
            }
        }
    }
    pairs
}
