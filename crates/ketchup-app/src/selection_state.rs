//! What is selected: parts, a group, exact faces and edges, or an underlay.

use crate::*;
use ketchup_model::underlay::UnderlayId;

/// An underlay selected alone, so Move and Rotate place it, and the objects of its
/// drawing picked in it, by id, so Delete takes them out.
#[derive(Clone, Debug)]
pub(crate) struct UnderlaySelection {
    pub(crate) id: UnderlayId,
    objects: BTreeSet<u32>,
}

/// No objects picked.
static NO_OBJECTS: BTreeSet<u32> = BTreeSet::new();

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
    pub(crate) fn select_underlay(&mut self, id: UnderlayId) {
        self.clear();
        self.underlay = Some(UnderlaySelection {
            id,
            objects: BTreeSet::new(),
        });
    }

    /// The underlay selected, if one is.
    pub(crate) fn underlay(&self) -> Option<UnderlayId> {
        self.underlay.as_ref().map(|selected| selected.id)
    }

    /// The objects of the selected underlay's drawing picked in it.
    pub(crate) fn underlay_objects(&self) -> &BTreeSet<u32> {
        self.underlay
            .as_ref()
            .map_or(&NO_OBJECTS, |selected| &selected.objects)
    }

    /// Picks `object` of the selected underlay's drawing; `additive` adds it to those
    /// picked, or takes it away when it was picked already.
    pub(crate) fn pick_underlay_object(&mut self, object: u32, additive: bool) {
        let Some(selected) = self.underlay.as_mut() else {
            return;
        };
        if !additive {
            selected.objects.clear();
        } else if selected.objects.remove(&object) {
            return;
        }
        selected.objects.insert(object);
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
