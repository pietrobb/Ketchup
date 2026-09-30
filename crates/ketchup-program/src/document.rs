//! Source-based program history. Only the current evaluated model is cached;
//! Undo and Redo evaluate the saved source and parameter values again.

use crate::{Evaluated, ProgramError, ProgramModel, Report, run};
use std::collections::{BTreeMap, VecDeque};

pub use ketchup_model::document::RuleProgramSource as ProgramSource;

pub const UNDO_LIMIT: usize = 10;

/// Part identities that the CAD publisher needs to create, update or remove.
/// Unlisted parts can retain their existing identities and exact geometry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartChanges {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
}

impl PartChanges {
    fn between(before: &ProgramModel, after: &ProgramModel) -> Self {
        let before: BTreeMap<_, _> = before
            .parts
            .iter()
            .map(|part| (part.name.as_str(), part))
            .collect();
        let after: BTreeMap<_, _> = after
            .parts
            .iter()
            .map(|part| (part.name.as_str(), part))
            .collect();
        let mut changes = Self::default();
        for (name, part) in &after {
            match before.get(name) {
                None => changes.added.push((*name).to_owned()),
                Some(old) if old != part => changes.modified.push((*name).to_owned()),
                Some(_) => {}
            }
        }
        changes.removed = before
            .keys()
            .filter(|name| !after.contains_key(*name))
            .map(|name| (*name).to_owned())
            .collect();
        changes
    }
}

pub struct ProgramDocument {
    versions: VecDeque<ProgramSource>,
    cursor: usize,
    evaluated: Evaluated,
    report: Report,
}

impl ProgramDocument {
    /// # Errors
    /// Returns the interpreter diagnostic without creating a document on failure.
    pub fn new(source: ProgramSource) -> Result<Self, ProgramError> {
        let (evaluated, report) = run(&source.file_name, &source.source, &source.overrides)?;
        Ok(Self {
            versions: VecDeque::from([source]),
            cursor: 0,
            evaluated,
            report,
        })
    }

    #[must_use]
    pub fn source(&self) -> &ProgramSource {
        &self.versions[self.cursor]
    }

    #[must_use]
    pub fn evaluated(&self) -> &Evaluated {
        &self.evaluated
    }

    #[must_use]
    pub fn report(&self) -> &Report {
        &self.report
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.versions.len()
    }

    /// Publishes one source edit, retaining at most ten reversible changes.
    /// Interpreter failure leaves the source, model and Redo branch untouched.
    /// Validation issues remain in the report; they do not prevent editing.
    ///
    /// # Errors
    /// Returns the interpreter diagnostic when the new source cannot run.
    pub fn replace(&mut self, source: ProgramSource) -> Result<PartChanges, ProgramError> {
        if self.source() == &source {
            return Ok(PartChanges::default());
        }
        let (evaluated, report) = run(&source.file_name, &source.source, &source.overrides)?;
        let changes = PartChanges::between(&self.evaluated.model, &evaluated.model);
        self.versions.truncate(self.cursor + 1);
        self.versions.push_back(source);
        if self.versions.len() > UNDO_LIMIT + 1 {
            self.versions.pop_front();
        }
        self.cursor = self.versions.len() - 1;
        self.evaluated = evaluated;
        self.report = report;
        Ok(changes)
    }

    /// # Errors
    /// Returns an interpreter error without moving the history cursor.
    pub fn undo(&mut self) -> Result<Option<PartChanges>, ProgramError> {
        if !self.can_undo() {
            return Ok(None);
        }
        self.restore(self.cursor - 1).map(Some)
    }

    /// # Errors
    /// Returns an interpreter error without moving the history cursor.
    pub fn redo(&mut self) -> Result<Option<PartChanges>, ProgramError> {
        if !self.can_redo() {
            return Ok(None);
        }
        self.restore(self.cursor + 1).map(Some)
    }

    fn restore(&mut self, cursor: usize) -> Result<PartChanges, ProgramError> {
        let source = &self.versions[cursor];
        let (evaluated, report) = run(&source.file_name, &source.source, &source.overrides)?;
        let changes = PartChanges::between(&self.evaluated.model, &evaluated.model);
        self.cursor = cursor;
        self.evaluated = evaluated;
        self.report = report;
        Ok(changes)
    }
}
