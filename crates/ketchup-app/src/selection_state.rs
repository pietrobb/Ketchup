//! What is selected: parts, a group, exact faces and edges, or an underlay.

use crate::*;

impl SelectionState {
    pub(crate) fn clear(&mut self) {
        self.occurrences.clear();
        self.primary = None;
        self.exact_references.clear();
        self.topological.clear();
        self.selected_group = None;
        self.underlay = None;
    }

    /// Selects the underlay `id` alone.
    pub(crate) fn select_underlay(&mut self, id: ketchup_model::underlay::UnderlayId) {
        self.clear();
        self.underlay = Some(id);
    }

    pub(crate) fn contains(&self, instance_path: &InstancePath) -> bool {
        self.occurrences.contains(instance_path)
    }

    pub(crate) fn select_exact(&mut self, selection: SelectionId, additive: bool) {
        self.topological.clear();
        self.underlay = None;
        let instance_path = selection.instance_path.clone();
        if additive && self.occurrences.contains(&instance_path) {
            self.occurrences.remove(&instance_path);
            self.exact_references.remove(&instance_path);
            if self
                .primary
                .as_ref()
                .is_some_and(|primary| primary.instance_path == selection.instance_path)
            {
                self.primary = self
                    .occurrences
                    .iter()
                    .find_map(|path| self.exact_references.get(path).cloned());
            }
            self.selected_group = None;
            return;
        }
        if !additive {
            self.occurrences.clear();
            self.exact_references.clear();
        }
        self.occurrences.insert(instance_path.clone());
        self.exact_references
            .insert(instance_path, selection.clone());
        if !additive || self.primary.is_none() {
            self.primary = Some(selection);
        }
        self.selected_group = None;
    }

    pub(crate) fn select_topological(
        &mut self,
        selection: SelectionId,
        topological: SnapshotBoundTopologicalSelection,
        additive: bool,
    ) -> bool {
        let target = topological.target();
        if additive {
            if self.topological.is_empty() {
                self.select_exact(selection.clone(), false);
                self.topological.push((selection, topological));
                return true;
            }
            if self.primary.as_ref().is_none_or(|primary| {
                primary.definition_id != selection.definition_id
                    || primary.instance_path != selection.instance_path
            }) || self.topological.first().is_some_and(|(_, existing)| {
                existing.target().reference.kind != target.reference.kind
                    || existing.target().reference.producer_feature_id
                        != target.reference.producer_feature_id
            }) {
                return false;
            }
            if let Some(index) = self
                .topological
                .iter()
                .position(|(_, existing)| existing.target() == target)
            {
                self.topological.remove(index);
                if self.topological.is_empty() {
                    self.occurrences.clear();
                    self.primary = None;
                    self.exact_references.clear();
                    self.selected_group = None;
                } else {
                    self.primary = Some(self.topological[0].0.clone());
                }
                return true;
            }
            if self.topological.len() == ketchup_model::tolerance::limits::FEATURE_REFERENCES {
                return false;
            }
            self.topological.push((selection, topological));
            self.topological.sort_by(|(_, left), (_, right)| {
                left.target().reference.cmp(&right.target().reference)
            });
            self.primary = Some(self.topological[0].0.clone());
            return true;
        }

        self.select_exact(selection.clone(), false);
        self.topological.push((selection, topological));
        true
    }

    pub(crate) fn select_path(&mut self, instance_path: InstancePath, additive: bool) {
        if additive && self.occurrences.contains(&instance_path) {
            self.occurrences.remove(&instance_path);
        } else {
            if !additive {
                self.occurrences.clear();
            }
            self.occurrences.insert(instance_path);
        }
        self.primary = None;
        self.exact_references.clear();
        self.topological.clear();
        self.selected_group = None;
        self.underlay = None;
    }

    pub(crate) fn select_occurrence(&mut self, occurrence_id: OccurrenceId, additive: bool) {
        self.select_path(InstancePath::root(occurrence_id), additive);
    }
}
