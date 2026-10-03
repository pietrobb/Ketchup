//! Publishes a whole Starlark program into the open window as one Undo step,
//! through the same reconciliation path as the headless session.
use super::*;
use ketchup_application::{RuleProgramApplyError, RuleProgramChange, SessionError};
use ketchup_model::document::RuleProgramSource;

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

fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("strings serialize")
}

fn session_error(error: WorkRecoveryMutationError<SessionError>) -> RuleProgramApplyError {
    RuleProgramApplyError::Session(match error {
        WorkRecoveryMutationError::Mutation(error) => error,
        WorkRecoveryMutationError::Recovery(error) => SessionError::Persistence(error),
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
    ) -> Result<
        (
            ProgramEdit,
            ketchup_program::Report,
            ketchup_program::ProgramModel,
        ),
        RuleProgramApplyError,
    > {
        self.apply_program_source_with(source, replace, Vec::new())
    }

    /// `apply_program_source` of a program that owns the document, with
    /// `also` (edits outside the program, e.g. removing a used-up drawn shape)
    /// in the same Undo step.
    pub(crate) fn apply_program_source_with(
        &mut self,
        source: RuleProgramSource,
        replace: bool,
        also: Vec<CanonicalCommand>,
    ) -> Result<
        (
            ProgramEdit,
            ketchup_program::Report,
            ketchup_program::ProgramModel,
        ),
        RuleProgramApplyError,
    > {
        let plan = ketchup_application::plan_rule_program(&self.document, &source)?;
        self.publish_program_plan(source, replace, also, plan)
    }

    pub(crate) fn publish_program_plan(
        &mut self,
        source: RuleProgramSource,
        replace: bool,
        also: Vec<CanonicalCommand>,
        plan: ketchup_application::RuleProgramPlan,
    ) -> Result<
        (
            ProgramEdit,
            ketchup_program::Report,
            ketchup_program::ProgramModel,
        ),
        RuleProgramApplyError,
    > {
        let change = match plan.change {
            RuleProgramChange::Unchanged | RuleProgramChange::SourceOnly if !also.is_empty() => {
                RuleProgramChange::Incremental(CommandBatch::new(Vec::new()))
            }
            RuleProgramChange::Incremental(_) | RuleProgramChange::Replacement
                if !also.is_empty() && self.document.current_rule_program().is_none() =>
            {
                return Err(RuleProgramApplyError::IncrementalUnsupported);
            }
            change => change,
        };
        let (edit, batch) = match change {
            RuleProgramChange::Unchanged => {
                return Ok((ProgramEdit::Unchanged, plan.report, plan.evaluated.model));
            }
            RuleProgramChange::SourceOnly => {
                self.complete_mutation_with_work_recovery(|document| {
                    document
                        .replace_rule_program_source(source)
                        .map_err(SessionError::Canonical)
                })
                .map_err(session_error)?;
                self.finish_program_edit();
                return Ok((ProgramEdit::SourceOnly, plan.report, plan.evaluated.model));
            }
            RuleProgramChange::Incremental(batch) => {
                let mut commands = batch.commands().to_vec();
                commands.extend(also);
                (ProgramEdit::Incremental, CommandBatch::new(commands))
            }
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
                let batch = ketchup_application::plan_rule_model_batch(
                    &self.document,
                    plan.evaluated.reference_model(),
                )?;
                (ProgramEdit::Created, batch)
            }
        };
        let proposal = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::rule_program())
            .map_err(|error| RuleProgramApplyError::Session(SessionError::Prepare(error)))?;
        self.complete_mutation_with_work_recovery(|document| {
            document
                .commit_verified_proposal(&proposal)
                .map_err(SessionError::Commit)?;
            document
                .bind_rule_program(source)
                .map_err(SessionError::Canonical)
        })
        .map_err(session_error)?;
        self.finish_program_edit();
        if edit == ProgramEdit::Created {
            self.camera.zoom_fit_pending = true;
        }
        Ok((edit, plan.report, plan.evaluated.model))
    }

    /// A fillet or chamfer on edges picked on a program-owned part is written
    /// into the program as `fillet(part, edges=[[a, b], ...], radius=r)` (or
    /// `chamfer(..., distance=d)`) and published like any program edit, so the
    /// program keeps owning the part. `None` when no program owns the part.
    pub(crate) fn program_general_finish(
        &mut self,
        source: &GeneralFinishSourcePlan,
        amount_mm: f64,
    ) -> Option<Result<(), Rejection>> {
        let (call, amount) = match source.kind {
            GeneralFinishKind::Fillet => ("fillet", "radius"),
            GeneralFinishKind::Chamfer => ("chamfer", "distance"),
            GeneralFinishKind::Shell => return None,
        };
        let program = self.document.current_rule_program()?.clone();
        let primary = source.source_primary.as_ref()?;
        let snapshot = self.document.current();
        let occurrence = snapshot.occurrence(primary.instance_path.root_occurrence())?;
        let owned = primary.instance_path.is_root()
            && ketchup_application::rule_program_part_sources(&program)
                .is_ok_and(|parts| parts.contains_key(occurrence.name()));
        if !owned {
            return None;
        }
        let mut edges = Vec::new();
        for selection in &source.topological_selections {
            let names = selection
                .resolve_current(&snapshot, &self.exact.topology_results)
                .ok()
                .and_then(|resolved| {
                    live_bridge::program_pick::edge_names(
                        self,
                        &snapshot,
                        &primary.instance_path,
                        &resolved.reference,
                    )
                });
            let Some((_, [first, second])) = names else {
                return Some(Err(Rejection::new(
                    "program.edge_unnamed",
                    RejectionPhase::Validation,
                )
                .target(occurrence.name().to_owned())
                .reason(format!(
                    "the picked edge of {:?} has no program name; round it with {call}() in the program",
                    occurrence.name()
                ))));
            };
            edges.push(format!("[{}, {}]", quote(&first), quote(&second)));
        }
        let mut rewritten = program;
        rewritten.source.push_str(&format!(
            "\n{call}({}, edges=[{}], {amount}={amount_mm})\n",
            quote(occurrence.name()),
            edges.join(", ")
        ));
        Some(
            self.apply_program_source(rewritten, false)
                .map(|_| ())
                .map_err(|error| failed("program.apply", error)),
        )
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
        let scene = snapshot.scene_query();
        let named = scene
            .iter()
            .filter_map(|part| {
                let name =
                    ketchup_application::rule_program_part_name(&snapshot, &part.instance_path)?;
                Some((name, part))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let parts = parts
            .iter()
            .map(|(name, lines)| {
                serde_json::json!({
                    "name": name,
                    "occurrence_id": named.get(name).filter(|part| part.instance_path.is_root()).map(|part| part.occurrence_id.0),
                    "instance_path": named.get(name).map(|part| &part.instance_path),
                    "definition_id": named.get(name).map(|part| part.definition_id.0),
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
            "groups": snapshot.groups().map(|group| serde_json::json!({
                "name": group.name(), "group_id": group.id().0,
                "parent_group_id": group.parent().map(|id| id.0),
                "occurrence_ids": snapshot.occurrences().filter(|part| part.parent() == Some(group.id())).map(|part| part.id().0).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "components": snapshot.definitions().filter(|definition| !definition.local_occurrence_ids().is_empty() || !definition.local_group_ids().is_empty()).map(|definition| serde_json::json!({
                "name": definition.name(), "definition_id": definition.id().0, "groups": Self::program_local_groups(&snapshot, definition.id()), "members": Self::program_local_members(&snapshot, definition.id()),
                "instances": scene.iter().filter(|part| part.definition_id == definition.id()).map(|part| serde_json::json!({
                    "name": ketchup_application::rule_program_part_name(&snapshot, &part.instance_path), "occurrence_id": part.instance_path.is_root().then_some(part.occurrence_id.0), "instance_path": part.instance_path, "transform": part.transform,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "evaluation_error": error,
        }))
    }
}
