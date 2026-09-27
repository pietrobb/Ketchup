//! Publishes a whole Starlark program into the open window as one Undo step,
//! through the same reconciliation path as the headless session.
use super::*;
use ketchup_application::{RuleProgramApplyError, RuleProgramChange, SessionError};
use ketchup_core::document::RuleProgramSource;

/// How a program edit changed the window's document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProgramEdit {
    Unchanged,
    SourceOnly,
    Incremental,
    Created,
}

impl ProgramEdit {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::SourceOnly => "source_only",
            Self::Incremental => "incremental",
            Self::Created => "created",
        }
    }
}

fn session_error(error: WorkRecoveryMutationError<SessionError>) -> RuleProgramApplyError {
    RuleProgramApplyError::Session(match error {
        WorkRecoveryMutationError::Mutation(error) => error,
        WorkRecoveryMutationError::Recovery(error) => SessionError::Persistence(error.to_string()),
    })
}

impl KetchupApp {
    /// Evaluates `source` and publishes only what changed, keeping every
    /// unchanged part's identity. A document no program owns is replaced only
    /// when it is empty, or when `replace` is set and nothing is unsaved.
    pub(crate) fn apply_program_source(
        &mut self,
        source: RuleProgramSource,
        replace: bool,
    ) -> Result<(ProgramEdit, ketchup_program::Report), RuleProgramApplyError> {
        let plan = ketchup_application::plan_rule_program(&self.document, &source)?;
        let (edit, batch) = match plan.change {
            RuleProgramChange::Unchanged => return Ok((ProgramEdit::Unchanged, plan.report)),
            RuleProgramChange::SourceOnly => {
                self.complete_mutation_with_work_recovery(|document| {
                    if document.replace_rule_program_source(source) {
                        Ok(())
                    } else {
                        Err(SessionError::Persistence(
                            "cannot replace rule program source".into(),
                        ))
                    }
                })
                .map_err(session_error)?;
                self.finish_program_edit();
                return Ok((ProgramEdit::SourceOnly, plan.report));
            }
            RuleProgramChange::Incremental(batch) => (ProgramEdit::Incremental, batch),
            RuleProgramChange::Replacement => {
                if self.document.current().definitions().next().is_some() {
                    if !replace {
                        return Err(RuleProgramApplyError::ReplacementConfirmationRequired);
                    }
                    if self.is_dirty() {
                        return Err(RuleProgramApplyError::UnsavedChanges);
                    }
                    self.new_document();
                }
                let batch = ketchup_application::plan_rule_part_batch(
                    &self.document,
                    &plan.evaluated.model.parts,
                )
                .map_err(|diagnostic| {
                    RuleProgramApplyError::Session(SessionError::Planning(diagnostic))
                })?;
                (ProgramEdit::Created, batch)
            }
        };
        let proposal = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::local_assistant_model())
            .map_err(|error| RuleProgramApplyError::Session(SessionError::Prepare(error)))?;
        self.complete_mutation_with_work_recovery(|document| {
            document
                .commit_verified_proposal(&proposal)
                .map_err(SessionError::Commit)?;
            if document.bind_rule_program(source) {
                Ok(())
            } else {
                Err(SessionError::Persistence(
                    "cannot bind rule program to revision".into(),
                ))
            }
        })
        .map_err(session_error)?;
        self.finish_program_edit();
        if edit == ProgramEdit::Created {
            self.zoom_fit_pending = true;
        }
        Ok((edit, plan.report))
    }

    fn finish_program_edit(&mut self) {
        self.invalidate_pending_import_reviews();
        self.clear_ephemeral_edit_state();
        self.reconcile_selection();
        self.status_key = "status-ready";
    }

    /// The program that owns the document with each part's occurrence and
    /// defining lines, or `None` when no program owns it.
    pub(crate) fn program_source_view(&self) -> Option<serde_json::Value> {
        let program = self.document.current_rule_program()?;
        let snapshot = self.document.current();
        let (parts, error) = match ketchup_application::rule_program_part_sources(program) {
            Ok(parts) => (parts, None),
            Err(error) => (Default::default(), Some(error)),
        };
        let parts = parts
            .iter()
            .map(|(name, lines)| {
                serde_json::json!({
                    "name": name,
                    "occurrence_id": snapshot
                        .occurrences()
                        .find(|occurrence| occurrence.name() == name)
                        .map(|occurrence| occurrence.id().0),
                    "lines": lines
                        .iter()
                        .map(|lines| [lines.first, lines.last])
                        .collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();
        Some(serde_json::json!({
            "file_name": program.file_name,
            "source": program.source,
            "overrides": program.overrides,
            "parts": parts,
            "evaluation_error": error,
        }))
    }
}
