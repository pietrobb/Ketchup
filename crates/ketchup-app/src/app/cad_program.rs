//! Typed CAD edit programs and validation context shared by the MCP bridge
//! and the window's own tools.

use crate::*;

impl KetchupApp {
    pub(crate) fn instance_path_label(path: &InstancePath) -> String {
        let mut label = path.root_occurrence().0.to_string();
        for step in path.steps() {
            match step {
                ketchup_model::document::InstancePathStep::Group(id) => {
                    label.push_str(&format!(" / G{}", id.0));
                }
                ketchup_model::document::InstancePathStep::Occurrence(id) => {
                    label.push_str(&format!(" / O{}", id.0));
                }
            }
        }
        label
    }

    pub(crate) fn validation_context(
        &self,
        snapshot: &Snapshot,
        exact_results: &ExactResultRegistry,
        selection: &AssistantValidationSelection,
    ) -> serde_json::Value {
        ketchup_application::validation::assistant_validation_context_with_worker(
            snapshot,
            exact_results,
            &ketchup_application::validation::AssistantValidationSelection {
                mode: selection.mode,
                requested: selection.requested.clone(),
                unknown: selection.unknown.clone(),
            },
            &self.file.container_data,
            self.validator_worker_path(),
            Duration::from_secs(30),
        )
    }

    pub fn plan_assistant_cad_edit_program(
        &self,
        program: &AssistantCadEditProgram,
    ) -> Result<CommandBatch, Box<AssistantRejectionDiagnostic>> {
        ketchup_application::plan_assistant_cad_edit_program(
            &self.document,
            &self.selected_occurrence_ids(),
            &self.exact.topology_results,
            program,
        )
    }

    pub(crate) fn derive_assistant_cad_edit_proposal(
        &self,
        program: &AssistantCadEditProgram,
    ) -> AssistantPlanningResult<Proposal> {
        let target = format!("document:{}", self.document.current().document_id().0);
        let batch = self.plan_assistant_cad_edit_program(program)?;
        self.document
            .prepare_proposal_with_context(batch, ProposalContext::local_assistant_model())
            .map_err(|error| {
                assistant_proposal_prepare_rejection(error, "cad_edit_program", &target)
            })
    }

    pub fn prepare_general_finish(
        &mut self,
        locator: TopologicalPickLocator,
        kind: GeneralFinishKind,
        amount_mm: f64,
    ) -> bool {
        if !amount_mm.is_finite() || amount_mm <= 0.0 || !self.select_topological_locator(locator) {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = match kind {
            GeneralFinishKind::Shell => ActiveTool::Shell,
            GeneralFinishKind::Fillet => ActiveTool::Fillet,
            GeneralFinishKind::Chamfer => ActiveTool::Chamfer,
        };
        self.value_box.input = amount_mm.to_string();
        self.refresh_general_finish_preview()
    }

    pub fn confirm_general_finish(&mut self) -> bool {
        // A scripted step answers in the same call.
        self.wait_for_program_evaluation();
        self.confirm_general_finish_preview()
    }

    /// Plans and commits one typed CAD program as one undo step, the way the
    /// MCP `edit propose` + `commit` pair does; lets headless tests build models.
    #[cfg(feature = "testing")]
    pub fn headless_apply_cad_program(
        &mut self,
        program: &AssistantCadEditProgram,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let proposal = self
            .derive_assistant_cad_edit_proposal(program)
            .map_err(|diagnostic| ketchup_rejection::Rejection::from(*diagnostic))?;
        self.commit_verified_proposal_with_work_recovery(&proposal)?;
        Ok(())
    }

    /// Runs the named validators (every validator when `validators` is empty)
    /// read-only, with the evidence MCP `validate` and the validator section in the dock read.
    #[cfg(feature = "testing")]
    pub fn headless_validation_context(
        &mut self,
        validators: &[&'static str],
    ) -> serde_json::Value {
        let snapshot = self.document.current();
        self.rebind_exact_results(&snapshot);
        let selection = AssistantValidationSelection {
            mode: if validators.is_empty() { "all" } else { "only" },
            requested: if validators.is_empty() {
                ASSISTANT_VALIDATOR_IDS.into_iter().collect()
            } else {
                validators.iter().copied().collect()
            },
            unknown: Vec::new(),
        };
        self.validation_context(&snapshot, &self.exact.results, &selection)
    }
}
