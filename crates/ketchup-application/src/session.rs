//! Stateful, GUI-independent canonical CAD authoring. All lengths are millimetres.
use crate::{
    evaluation::*,
    plan_assistant_cad_edit_program,
    validation::{AssistantValidationSelection, assistant_validation_context_with_worker},
};
use ketchup_core::assistant_sidecar::{AssistantCadEditProgram, AssistantRejectionDiagnostic};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DocumentStore, OccurrenceId, Proposal,
    ProposalCommitError, ProposalContext, ProposalPrepareError, Snapshot, VerifiedProposalCommit,
};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_core::persistence::{self, ContainerData};
use std::{
    collections::BTreeSet,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryState {
    requested_path: PathBuf,
    source_path: PathBuf,
}
impl RecoveryState {
    #[must_use]
    pub fn requested_path(&self) -> &Path {
        &self.requested_path
    }
    #[must_use]
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }
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
    recovery: Option<RecoveryState>,
    file_identity: Option<persistence::FileIdentity>,
    work_recovery_identity: Option<persistence::FileIdentity>,
    pending_work_recovery_cleanup: Option<(PathBuf, persistence::FileIdentity)>,
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
            recovery: None,
            file_identity: None,
            work_recovery_identity: None,
            pending_work_recovery_cleanup: None,
            saved_digest: None,
            exact_results: ExactResultRegistry::default(),
            topology_results: ExactResultRegistry::default(),
        }
    }
    /// Review-only and invalid input never replaces a live session.
    pub fn open(path: impl AsRef<Path>, settings: SessionSettings) -> Result<Self, SessionError> {
        let requested_path = path.as_ref();
        let loaded = persistence::load_file_with_source(requested_path)
            .map_err(|error| SessionError::Persistence(error.to_string()))?;
        let (outcome, source_path, source_bytes, work_recovery_identity) = loaded.into_parts();
        let (document, container_data) = outcome
            .into_editable_with_container()
            .map_err(|_| SessionError::ReviewOnly)?;
        let recovery = (source_path != requested_path).then(|| RecoveryState {
            requested_path: requested_path.to_owned(),
            source_path,
        });
        let saved_digest = Some(document.history_digest());
        Ok(Self {
            document,
            container_data,
            settings,
            path: recovery.is_none().then(|| requested_path.to_owned()),
            file_identity: recovery
                .is_none()
                .then(|| persistence::FileIdentity::from_bytes(&source_bytes)),
            work_recovery_identity,
            pending_work_recovery_cleanup: None,
            recovery,
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
        self.save_with_history(path.as_ref(), options, true)
    }
    /// Explicitly saves only the current revision after discarding persistent Undo/Redo history.
    /// The live history is truncated only after the atomic or no-clobber write succeeds.
    pub fn save_current_snapshot(
        &mut self,
        path: impl AsRef<Path>,
        options: SaveOptions,
    ) -> Result<(), SessionError> {
        self.save_with_history(path.as_ref(), options, false)
    }
    fn save_with_history(
        &mut self,
        path: &Path,
        options: SaveOptions,
        preserve_history: bool,
    ) -> Result<(), SessionError> {
        self.retry_pending_work_recovery_cleanup()?;
        let saved_bytes = if preserve_history {
            persistence::save_document_store(&self.document, &self.container_data)
        } else {
            persistence::save_document_store_current_snapshot(&self.document, &self.container_data)
        }
        .map_err(|error| SessionError::Persistence(error.to_string()))?;
        let saved_identity = persistence::FileIdentity::from_bytes(&saved_bytes);
        let expected_identity = (self.path.as_deref() == Some(path))
            .then_some(self.file_identity)
            .flatten();
        if options.overwrite {
            let result = match (preserve_history, expected_identity) {
                (true, Some(expected)) => {
                    persistence::save_atomic_document_store_with_container_if_unchanged(
                        path,
                        &self.document,
                        &self.container_data,
                        expected,
                    )
                    .map(|_| ())
                }
                (false, Some(expected)) => {
                    persistence::save_atomic_document_store_current_snapshot_with_container_if_unchanged(
                        path,
                        &self.document,
                        &self.container_data,
                        expected,
                    )
                    .map(|_| ())
                }
                (true, None) => persistence::save_atomic_document_store_with_container(
                    path,
                    &self.document,
                    &self.container_data,
                ),
                (false, None) => {
                    persistence::save_atomic_document_store_current_snapshot_with_container(
                        path,
                        &self.document,
                        &self.container_data,
                    )
                }
            };
            result.map_err(|error| SessionError::Persistence(error.to_string()))?;
        } else {
            let result = if preserve_history {
                persistence::save_atomic_document_store_with_container_if_absent(
                    path,
                    &self.document,
                    &self.container_data,
                )
            } else {
                persistence::save_atomic_document_store_current_snapshot_with_container_if_absent(
                    path,
                    &self.document,
                    &self.container_data,
                )
            };
            result.map_err(|error| SessionError::Persistence(error.to_string()))?;
        }
        if !preserve_history {
            self.document.discard_history_before_current();
        }
        let owned_recovery_path = self.path.clone().or_else(|| {
            self.recovery
                .as_ref()
                .map(|recovery| recovery.requested_path.clone())
        });
        if let (Some(recovery_path), Some(recovery_identity)) =
            (owned_recovery_path, self.work_recovery_identity)
            && persistence::clear_work_recovery(&recovery_path, Some(recovery_identity)).is_err()
        {
            self.pending_work_recovery_cleanup = Some((recovery_path, recovery_identity));
        }
        self.path = Some(path.to_owned());
        self.recovery = None;
        self.file_identity = Some(saved_identity);
        self.work_recovery_identity = None;
        self.saved_digest = Some(self.document.history_digest());
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
    pub fn recovery_state(&self) -> Option<&RecoveryState> {
        self.recovery.as_ref()
    }
    fn retry_pending_work_recovery_cleanup(&mut self) -> Result<(), SessionError> {
        let Some((path, identity)) = self.pending_work_recovery_cleanup.clone() else {
            return Ok(());
        };
        persistence::clear_work_recovery(&path, Some(identity))
            .map_err(|error| SessionError::Persistence(error.to_string()))?;
        self.pending_work_recovery_cleanup = None;
        Ok(())
    }
    pub fn write_work_recovery_checkpoint(&mut self) -> Result<bool, SessionError> {
        let _ = self.retry_pending_work_recovery_cleanup();
        let identity = Self::write_work_recovery_checkpoint_for(
            &self.document,
            &self.container_data,
            self.path.as_deref(),
            self.file_identity,
            self.saved_digest.as_deref(),
            self.work_recovery_identity,
        )?;
        let written = identity.is_some();
        self.work_recovery_identity = identity;
        Ok(written)
    }
    fn write_work_recovery_checkpoint_for(
        document: &DocumentStore,
        container_data: &ContainerData,
        path: Option<&Path>,
        file_identity: Option<persistence::FileIdentity>,
        saved_digest: Option<&str>,
        work_recovery_identity: Option<persistence::FileIdentity>,
    ) -> Result<Option<persistence::FileIdentity>, SessionError> {
        let (Some(path), Some(identity)) = (path, file_identity) else {
            return Ok(work_recovery_identity);
        };
        let current_digest = document.history_digest();
        if saved_digest == Some(current_digest.as_str()) {
            persistence::clear_work_recovery(path, work_recovery_identity)
                .map_err(|error| SessionError::Persistence(error.to_string()))?;
            return Ok(None);
        }
        persistence::save_work_recovery_document_store_with_container(
            path,
            document,
            container_data,
            identity,
        )
        .map(Some)
        .map_err(|error| SessionError::Persistence(error.to_string()))
    }
    pub fn is_modified(&self) -> bool {
        self.recovery.is_some()
            || self.saved_digest.as_ref() != Some(&self.document.history_digest())
    }
    pub fn visible_undo_steps(&self) -> usize {
        self.document.visible_undo_steps()
    }
    pub fn visible_redo_steps(&self) -> usize {
        self.document.visible_redo_steps()
    }
    pub fn mutation_epoch(&self) -> u64 {
        self.document.mutation_epoch()
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
    fn mutate_with_work_recovery<T>(
        &mut self,
        mutate: impl FnOnce(&mut DocumentStore) -> Result<T, SessionError>,
    ) -> Result<T, SessionError> {
        let container_data = &self.container_data;
        let path = self.path.as_deref();
        let file_identity = self.file_identity;
        let saved_digest = self.saved_digest.as_deref();
        let work_recovery_identity = self.work_recovery_identity;
        let mut next_work_recovery_identity = work_recovery_identity;
        let result = self.document.try_canonical_transaction(mutate, |document| {
            next_work_recovery_identity = Self::write_work_recovery_checkpoint_for(
                document,
                container_data,
                path,
                file_identity,
                saved_digest,
                work_recovery_identity,
            )?;
            Ok(())
        });
        if result.is_ok() {
            self.work_recovery_identity = next_work_recovery_identity;
        }
        result
    }
    pub fn apply_proposal(&mut self, proposal: &Proposal) -> Result<Snapshot, SessionError> {
        self.apply_proposal_verified(proposal)?;
        Ok(self.snapshot())
    }
    pub fn apply_proposal_verified(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, SessionError> {
        let committed = self.mutate_with_work_recovery(|document| {
            document
                .commit_verified_proposal(proposal)
                .map_err(SessionError::Commit)
        })?;
        self.rebind();
        Ok(committed)
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
        let snapshot =
            self.mutate_with_work_recovery(|document| document.undo().ok_or(SessionError::NoUndo))?;
        self.rebind();
        Ok(snapshot)
    }
    pub fn redo(&mut self) -> Result<Snapshot, SessionError> {
        let snapshot =
            self.mutate_with_work_recovery(|document| document.redo().ok_or(SessionError::NoRedo))?;
        self.rebind();
        Ok(snapshot)
    }
    pub fn start_exact_evaluation_task(&mut self) -> ExactEvaluationTask {
        self.start_scoped_exact_evaluation_task(None)
    }
    pub fn start_scoped_exact_evaluation_task(
        &mut self,
        scope: Option<&BTreeSet<ProducerKey>>,
    ) -> ExactEvaluationTask {
        self.rebind();
        let path = self.settings.exact_worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        start_exact_evaluation_scoped(
            self.snapshot(),
            &self.container_data,
            &self.exact_results,
            &self.topology_results,
            path,
            scope,
            || {},
        )
    }
    pub fn publish_exact_evaluation(
        &mut self,
        task: &ExactEvaluationTask,
        products: ExactEvaluationProducts,
    ) -> Result<EvaluationReport, SessionError> {
        let mut exact_results = self.exact_results.clone();
        let mut topology_results = self.topology_results.clone();
        let report = self.mutate_with_work_recovery(|document| {
            publish_exact_products(
                document,
                &mut exact_results,
                &mut topology_results,
                task,
                products,
            )
            .map_err(SessionError::Evaluation)
        })?;
        self.exact_results = exact_results;
        self.topology_results = topology_results;
        Ok(report)
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
        let task = self.start_exact_evaluation_task();
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
        self.publish_exact_evaluation(&task, products)
    }
    pub fn validators(&self, selection: &AssistantValidationSelection) -> serde_json::Value {
        assistant_validation_context_with_worker(
            &self.snapshot(),
            &self.exact_results,
            selection,
            &self.container_data,
            self.settings.exact_worker_path.clone(),
            self.settings.evaluation_timeout,
        )
    }
}
