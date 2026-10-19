//! Says why an edit detached the program: the first part it changed and what
//! of that change the program cannot write.
use super::*;
use crate::program_evaluation::Lookup;
use ketchup_model::document::{Feature, Occurrence, RuleProgramSource};

/// The detaching edit is undone only while it is still the current revision;
/// any later change closes the notice.
pub(crate) struct ProgramDetachNotice {
    pub(crate) revision_id: u64,
    pub(crate) reason: String,
    /// The notice's text; after deleting every part it says none stay.
    pub(crate) body: String,
}

/// The features that shape an occurrence's definition.
fn shape(snapshot: &Snapshot, occurrence: &Occurrence) -> Option<Vec<Feature>> {
    snapshot.definition(occurrence.definition_id()).map(|made| {
        (made.feature_ids().iter())
            .filter_map(|id| snapshot.feature(*id).cloned())
            .collect()
    })
}

impl KetchupApp {
    /// The notice for the edit from `before`, which `program` owned, to the
    /// current document.
    pub(crate) fn program_detach_notice(
        &self,
        before: &Snapshot,
        program: RuleProgramSource,
    ) -> ProgramDetachNotice {
        let emptied = self.document.current().occurrences().next().is_none();
        let (reason, body_key) = if emptied {
            let reason = self.catalog.text("program-detached-reason-emptied");
            (reason, "program-detached-body-emptied")
        } else {
            let reason = self.program_detach_reason(before, &program);
            (reason, "program-detached-body")
        };
        let arguments = BTreeMap::from([("file", program.file_name), ("reason", reason.clone())]);
        ProgramDetachNotice {
            body: self.catalog.format(body_key, &arguments),
            revision_id: self.document.current().revision_id(),
            reason,
        }
    }

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
