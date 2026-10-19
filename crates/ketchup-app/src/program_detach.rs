//! Says why an edit detached the program: the first part it changed and what
//! of that change the program cannot write.
use super::*;
use crate::program_evaluation::Lookup;
use ketchup_model::document::{Feature, Occurrence, RuleProgramSource};

/// The features that shape an occurrence's definition.
fn shape(snapshot: &Snapshot, occurrence: &Occurrence) -> Option<Vec<Feature>> {
    snapshot.definition(occurrence.definition_id()).map(|made| {
        (made.feature_ids().iter())
            .filter_map(|id| snapshot.feature(*id).cloned())
            .collect()
    })
}

impl KetchupApp {
    /// Why the edit from `before`, which `program` owned, to the current
    /// document detached the program, as the end of a sentence.
    pub(crate) fn program_detach_reason(
        &self,
        before: &Snapshot,
        program: &RuleProgramSource,
    ) -> String {
        let after = self.document.current();
        let made = |occurrence: &Occurrence| match self.program_evaluations.try_get(program) {
            Lookup::Ready(Ok(evaluated)) => ketchup_application::rule_program_part_name(
                before,
                &InstancePath::root(occurrence.id()),
            )
            .is_some_and(|name| evaluated.model.part(&name).is_some()),
            _ => true,
        };
        let reason = |key: &str, part: &str| {
            self.catalog
                .format(key, &BTreeMap::from([("part", part.to_owned())]))
        };
        for new in after.occurrences() {
            let Some(old) = before.occurrence(new.id()) else {
                return reason("program-detached-reason-added", new.name());
            };
            let (old_shape, new_shape) = (shape(before, old), shape(&after, new));
            if old == new && old_shape == new_shape {
                continue;
            }
            let key = if !made(old) {
                "program-detached-reason-foreign"
            } else if !new.transform().is_rigid() {
                "program-detached-reason-scaled"
            } else if old_shape != new_shape {
                "program-detached-reason-shape"
            } else {
                "program-detached-reason-edit"
            };
            return reason(key, old.name());
        }
        match before
            .occurrences()
            .find(|old| after.occurrence(old.id()).is_none())
        {
            Some(old) if !made(old) => reason("program-detached-reason-foreign", old.name()),
            Some(old) => reason("program-detached-reason-edit", old.name()),
            None => self.catalog.text("program-detached-reason-other"),
        }
    }
}
