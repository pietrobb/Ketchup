//! Stateful, GUI-independent canonical CAD authoring. All lengths are millimetres.
use crate::{
    evaluation::*,
    plan_assistant_cad_edit_program,
    validation::{AssistantValidationSelection, assistant_validation_context},
};
use ketchup_core::assistant_sidecar::{AssistantCadEditProgram, AssistantRejectionDiagnostic};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DocumentStore, OccurrenceId, Proposal,
    ProposalCommitError, ProposalContext, ProposalPrepareError, Snapshot,
};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_core::persistence::{self, ContainerData};
use std::{
    collections::BTreeSet,
    io::Write as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct SessionSettings {
    /// None discovers the verified worker beside this executable or its parent.
    pub exact_worker_path: Option<PathBuf>,
    pub evaluation_timeout: Duration,
}
impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            exact_worker_path: None,
            evaluation_timeout: Duration::from_secs(30),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SaveOptions {
    pub overwrite: bool,
}
#[derive(Debug)]
pub enum SessionError {
    Planning(Box<AssistantRejectionDiagnostic>),
    Prepare(ProposalPrepareError),
    Commit(ProposalCommitError),
    Canonical(CanonicalError),
    Persistence(String),
    ReviewOnly,
    NoUndo,
    NoRedo,
    Evaluation(String),
}
impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SessionError {}

pub struct DocumentSession {
    document: DocumentStore,
    container_data: ContainerData,
    settings: SessionSettings,
    path: Option<PathBuf>,
    saved_digest: Option<String>,
    exact_results: ExactResultRegistry,
    topology_results: ExactResultRegistry,
}
impl Default for DocumentSession {
    fn default() -> Self {
        Self::new(SessionSettings::default())
    }
}
impl DocumentSession {
    pub fn new(settings: SessionSettings) -> Self {
        Self {
            document: DocumentStore::new(),
            container_data: ContainerData::default(),
            settings,
            path: None,
            saved_digest: None,
            exact_results: ExactResultRegistry::default(),
            topology_results: ExactResultRegistry::default(),
        }
    }
    /// Review-only and invalid input never replaces a live session.
    pub fn open(path: impl AsRef<Path>, settings: SessionSettings) -> Result<Self, SessionError> {
        let outcome = persistence::load_file(path.as_ref())
            .map_err(|error| SessionError::Persistence(error.to_string()))?;
        let (document, container_data) = outcome
            .into_editable_with_container()
            .map_err(|_| SessionError::ReviewOnly)?;
        let saved_digest = Some(document.current().canonical_digest());
        Ok(Self {
            document,
            container_data,
            settings,
            path: Some(path.as_ref().to_owned()),
            saved_digest,
            exact_results: ExactResultRegistry::default(),
            topology_results: ExactResultRegistry::default(),
        })
    }
    /// No-clobber publication is atomic, including when another process creates the destination.
    /// Explicit overwrite uses the core's atomic save/recovery implementation.
    pub fn save(
        &mut self,
        path: impl AsRef<Path>,
        options: SaveOptions,
    ) -> Result<(), SessionError> {
        let path = path.as_ref();
        let snapshot = self.snapshot();
        if options.overwrite {
            persistence::save_atomic_with_container(path, &snapshot, &self.container_data)
                .map_err(|error| SessionError::Persistence(error.to_string()))?;
        } else {
            let bytes = persistence::save_container(&snapshot, &self.container_data)
                .map_err(|error| SessionError::Persistence(error.to_string()))?;
            persistence::load(&bytes)
                .map_err(|error| SessionError::Persistence(error.to_string()))?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut temporary = tempfile::NamedTempFile::new_in(parent)
                .map_err(|error| SessionError::Persistence(error.to_string()))?;
            temporary
                .write_all(&bytes)
                .and_then(|()| temporary.as_file_mut().sync_all())
                .map_err(|error| SessionError::Persistence(error.to_string()))?;
            temporary
                .persist_noclobber(path)
                .map_err(|error| SessionError::Persistence(error.to_string()))?;
        }
        self.path = Some(path.to_owned());
        self.saved_digest = Some(snapshot.canonical_digest());
        Ok(())
    }
    pub fn snapshot(&self) -> Snapshot {
        self.document.current()
    }
    pub fn container_data(&self) -> &ContainerData {
        &self.container_data
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn is_modified(&self) -> bool {
        self.saved_digest.as_ref() != Some(&self.snapshot().canonical_digest())
    }
    pub fn visible_undo_steps(&self) -> usize {
        self.document.visible_undo_steps()
    }
    pub fn visible_redo_steps(&self) -> usize {
        self.document.visible_redo_steps()
    }
    pub fn exact_results(&self) -> &ExactResultRegistry {
        &self.exact_results
    }
    pub fn topology_results(&self) -> &ExactResultRegistry {
        &self.topology_results
    }
    /// Observational planning. The returned proposal retains the core's revision and policy checks.
    pub fn plan_cad_program(
        &self,
        program: &AssistantCadEditProgram,
        selection: &BTreeSet<OccurrenceId>,
    ) -> Result<Proposal, SessionError> {
        let batch = plan_assistant_cad_edit_program(
            &self.document,
            selection,
            &self.topology_results,
            program,
        )
        .map_err(SessionError::Planning)?;
        self.plan_commands(batch)
    }
    /// Generic canonical operations, including grounding, use the same proposal gate.
    /// No human confirmation or high-risk authorization is synthesized.
    pub fn plan_commands(&self, batch: CommandBatch) -> Result<Proposal, SessionError> {
        self.document
            .prepare_proposal_with_context(batch, ProposalContext::local_assistant_model())
            .map_err(SessionError::Prepare)
    }
    pub fn apply_proposal(&mut self, proposal: &Proposal) -> Result<Snapshot, SessionError> {
        self.document
            .commit_verified_proposal(proposal)
            .map_err(SessionError::Commit)?;
        self.rebind();
        Ok(self.snapshot())
    }
    /// One bounded program is one Undo step. Created IDs can be obtained by snapshot diff.
    pub fn apply_cad_program(
        &mut self,
        program: &AssistantCadEditProgram,
        selection: &BTreeSet<OccurrenceId>,
    ) -> Result<Snapshot, SessionError> {
        let proposal = self.plan_cad_program(program, selection)?;
        self.apply_proposal(&proposal)
    }
    pub fn set_grounded(
        &mut self,
        id: OccurrenceId,
        grounded: bool,
    ) -> Result<Snapshot, SessionError> {
        let proposal = self.plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded { id, grounded },
        ]))?;
        self.apply_proposal(&proposal)
    }
    fn rebind(&mut self) {
        rebind_exact_results(
            &self.snapshot(),
            &mut self.exact_results,
            &mut self.topology_results,
        );
    }
    pub fn undo(&mut self) -> Result<Snapshot, SessionError> {
        self.document.undo().ok_or(SessionError::NoUndo)?;
        self.rebind();
        Ok(self.snapshot())
    }
    pub fn redo(&mut self) -> Result<Snapshot, SessionError> {
        self.document.redo().ok_or(SessionError::NoRedo)?;
        self.rebind();
        Ok(self.snapshot())
    }
    pub fn evaluate(&mut self) -> Result<EvaluationReport, SessionError> {
        self.evaluate_with_timeout(self.settings.evaluation_timeout)
    }
    /// Evaluates with a per-call budget including preparation and waiting, checked before publication.
    /// Zero refuses evaluation without changing state, even when results are already current.
    pub fn evaluate_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<EvaluationReport, SessionError> {
        let started = Instant::now();
        if timeout.is_zero() {
            return Err(SessionError::Evaluation(
                "exact evaluation timed out".into(),
            ));
        }
        self.rebind();
        let path = self.settings.exact_worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        let task = start_exact_evaluation(
            self.snapshot(),
            &self.container_data,
            &self.exact_results,
            &self.topology_results,
            path,
            || {},
        );
        let products = task
            .wait(
                timeout
                    // Preparation consumes the same budget as worker execution.
                    .saturating_sub(started.elapsed()),
            )
            .map_err(SessionError::Evaluation)?;
        if started.elapsed() >= timeout {
            task.cancel();
            return Err(SessionError::Evaluation(
                "exact evaluation timed out".into(),
            ));
        }
        publish_exact_products(
            &mut self.document,
            &mut self.exact_results,
            &mut self.topology_results,
            &task,
            products,
        )
        .map_err(SessionError::Evaluation)
    }
    pub fn validators(&self, selection: &AssistantValidationSelection) -> serde_json::Value {
        assistant_validation_context(&self.snapshot(), &self.exact_results, selection)
    }
}
