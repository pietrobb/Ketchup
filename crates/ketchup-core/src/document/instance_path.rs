use super::{LocalGroupId, LocalOccurrenceId, OccurrenceId};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstancePath {
    root: OccurrenceId,
    pub(super) steps: Vec<InstancePathStep>,
}

impl InstancePath {
    #[must_use]
    pub const fn root(root: OccurrenceId) -> Self {
        Self {
            root,
            steps: Vec::new(),
        }
    }

    #[must_use]
    pub const fn root_occurrence(&self) -> OccurrenceId {
        self.root
    }

    #[must_use]
    pub fn steps(&self) -> &[InstancePathStep] {
        &self.steps
    }

    #[must_use]
    pub fn with_step(&self, step: InstancePathStep) -> Self {
        let mut path = self.clone();
        path.steps.push(step);
        path
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.steps.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InstancePathStep {
    Group(LocalGroupId),
    Occurrence(LocalOccurrenceId),
}
