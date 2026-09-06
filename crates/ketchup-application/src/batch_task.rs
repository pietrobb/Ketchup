use crate::model_query::{ModelQuery, QueryError};
use crate::{DocumentSession, SessionError};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DocumentStore, OccurrenceId, Proposal, ProposalBudget,
    ProposalContext, Snapshot, VerifiedProposalCommit,
};
use serde::{Deserialize, Serialize};

pub const MAX_OCCURRENCE_BATCH_ITEMS: usize = ProposalBudget::HOST_MAX.max_commands;
const BATCH_RECEIPT_SCHEMA: &str = "ketchup.occurrence-batch-receipt.v1";

fn deserialize_required_color<'de, D>(deserializer: D) -> Result<Option<[u8; 3]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::deserialize(deserializer)
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OccurrenceBatchOperation {
    SetColor {
        #[serde(deserialize_with = "deserialize_required_color")]
        color: Option<[u8; 3]>,
    },
}

impl OccurrenceBatchOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::SetColor { .. } => "set_color",
        }
    }

    fn command(self, id: OccurrenceId) -> CanonicalCommand {
        match self {
            Self::SetColor { color } => CanonicalCommand::SetOccurrenceColor { id, color },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BatchDocumentStamp {
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub mutation_epoch: u64,
}

impl BatchDocumentStamp {
    fn from_host(host: &(impl OccurrenceBatchDocument + ?Sized)) -> Self {
        let snapshot = host.batch_snapshot();
        Self {
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            mutation_epoch: host.batch_mutation_epoch(),
        }
    }
}

pub trait OccurrenceBatchDocument {
    fn batch_snapshot(&self) -> Snapshot;
    fn batch_mutation_epoch(&self) -> u64;
    fn batch_plan(&self, batch: CommandBatch) -> Result<Proposal, OccurrenceBatchError>;
    fn batch_commit(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, OccurrenceBatchError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OccurrenceBatchReceipt {
    pub schema: &'static str,
    pub workset_handle: String,
    pub scope: &'static str,
    pub operation: &'static str,
    pub operation_payload: OccurrenceBatchOperation,
    pub batch_index: usize,
    pub applied_count: usize,
    pub total_count: usize,
    pub completed_count: usize,
    pub remaining_count: usize,
    pub complete: bool,
    pub before: BatchDocumentStamp,
    pub after: BatchDocumentStamp,
    pub command_digest: String,
    pub result_digest: String,
    pub verified_write_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceBatchState {
    Pending,
    Running,
    Completed,
    Cancelled,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OccurrenceBatchStatus {
    pub schema: &'static str,
    pub state: OccurrenceBatchState,
    pub workset_handle: String,
    pub scope: &'static str,
    pub operation: &'static str,
    pub operation_payload: OccurrenceBatchOperation,
    pub total_count: usize,
    pub completed_count: usize,
    pub remaining_count: usize,
    pub completed_batches: usize,
    pub expected: BatchDocumentStamp,
    pub actual: BatchDocumentStamp,
}

#[derive(Debug)]
pub enum OccurrenceBatchError {
    Query(QueryError),
    Session(SessionError),
    HostTransaction,
    Cancelled,
    StaleTask {
        expected: BatchDocumentStamp,
        actual: BatchDocumentStamp,
    },
}

impl OccurrenceBatchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Query(error) => error.code(),
            Self::Session(_) | Self::HostTransaction => "batch_transaction_failed",
            Self::Cancelled => "batch_cancelled",
            Self::StaleTask { .. } => "stale_batch_task",
        }
    }
}

impl From<QueryError> for OccurrenceBatchError {
    fn from(error: QueryError) -> Self {
        Self::Query(error)
    }
}

impl From<SessionError> for OccurrenceBatchError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl OccurrenceBatchDocument for DocumentSession {
    fn batch_snapshot(&self) -> Snapshot {
        self.snapshot()
    }

    fn batch_mutation_epoch(&self) -> u64 {
        self.mutation_epoch()
    }

    fn batch_plan(&self, batch: CommandBatch) -> Result<Proposal, OccurrenceBatchError> {
        Ok(self.plan_commands(batch)?)
    }

    fn batch_commit(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, OccurrenceBatchError> {
        Ok(self.apply_proposal_verified(proposal)?)
    }
}

impl OccurrenceBatchDocument for DocumentStore {
    fn batch_snapshot(&self) -> Snapshot {
        self.current()
    }

    fn batch_mutation_epoch(&self) -> u64 {
        self.mutation_epoch()
    }

    fn batch_plan(&self, batch: CommandBatch) -> Result<Proposal, OccurrenceBatchError> {
        self.prepare_proposal_with_context(batch, ProposalContext::local_assistant_model())
            .map_err(|_| OccurrenceBatchError::HostTransaction)
    }

    fn batch_commit(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, OccurrenceBatchError> {
        self.commit_verified_proposal(proposal)
            .map_err(|_| OccurrenceBatchError::HostTransaction)
    }
}

pub struct OccurrenceBatchTask {
    workset_handle: String,
    operation: OccurrenceBatchOperation,
    occurrence_ids: Vec<OccurrenceId>,
    completed_count: usize,
    completed_batches: usize,
    expected: BatchDocumentStamp,
    cancelled: bool,
}

impl ModelQuery {
    pub fn create_occurrence_batch_task(
        &self,
        host: &(impl OccurrenceBatchDocument + ?Sized),
        workset_handle: &str,
        operation: OccurrenceBatchOperation,
    ) -> Result<OccurrenceBatchTask, QueryError> {
        let snapshot = host.batch_snapshot();
        Ok(OccurrenceBatchTask {
            workset_handle: workset_handle.to_owned(),
            operation,
            occurrence_ids: self.workset_occurrence_ids(&snapshot, workset_handle)?,
            completed_count: 0,
            completed_batches: 0,
            expected: BatchDocumentStamp::from_host(host),
            cancelled: false,
        })
    }
}

impl OccurrenceBatchTask {
    pub fn status(&self, host: &(impl OccurrenceBatchDocument + ?Sized)) -> OccurrenceBatchStatus {
        let actual = BatchDocumentStamp::from_host(host);
        let state = if self.is_complete() {
            OccurrenceBatchState::Completed
        } else if self.cancelled {
            OccurrenceBatchState::Cancelled
        } else if self.expected != actual {
            OccurrenceBatchState::Stale
        } else if self.completed_count == 0 {
            OccurrenceBatchState::Pending
        } else {
            OccurrenceBatchState::Running
        };
        OccurrenceBatchStatus {
            schema: "ketchup.occurrence-batch-status.v1",
            state,
            workset_handle: self.workset_handle.clone(),
            scope: "occurrences",
            operation: self.operation.name(),
            operation_payload: self.operation,
            total_count: self.occurrence_ids.len(),
            completed_count: self.completed_count,
            remaining_count: self.occurrence_ids.len() - self.completed_count,
            completed_batches: self.completed_batches,
            expected: self.expected.clone(),
            actual,
        }
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn is_complete(&self) -> bool {
        self.completed_count == self.occurrence_ids.len()
    }

    pub fn total_count(&self) -> usize {
        self.occurrence_ids.len()
    }

    pub const fn completed_count(&self) -> usize {
        self.completed_count
    }

    pub fn commit_next(
        &mut self,
        host: &mut (impl OccurrenceBatchDocument + ?Sized),
    ) -> Result<Option<OccurrenceBatchReceipt>, OccurrenceBatchError> {
        self.commit_next_with_cancel(host, || false)
    }

    /// Cancellation linearizes at the second poll, immediately before the atomic commit.
    /// A request arriving after that poll applies to the next bounded transaction.
    pub fn commit_next_with_cancel(
        &mut self,
        host: &mut (impl OccurrenceBatchDocument + ?Sized),
        mut cancellation_requested: impl FnMut() -> bool,
    ) -> Result<Option<OccurrenceBatchReceipt>, OccurrenceBatchError> {
        if self.is_complete() {
            return Ok(None);
        }
        if self.cancelled || cancellation_requested() {
            self.cancelled = true;
            return Err(OccurrenceBatchError::Cancelled);
        }

        let actual = BatchDocumentStamp::from_host(host);
        if actual != self.expected {
            return Err(OccurrenceBatchError::StaleTask {
                expected: self.expected.clone(),
                actual,
            });
        }

        let end = self
            .completed_count
            .saturating_add(MAX_OCCURRENCE_BATCH_ITEMS)
            .min(self.occurrence_ids.len());
        let commands = self.occurrence_ids[self.completed_count..end]
            .iter()
            .copied()
            .map(|id| self.operation.command(id))
            .collect();
        let proposal = host.batch_plan(CommandBatch::new(commands))?;

        if cancellation_requested() {
            self.cancelled = true;
            return Err(OccurrenceBatchError::Cancelled);
        }

        let before = self.expected.clone();
        let command_digest = proposal.command_digest().to_owned();
        let result_digest = proposal.intended_result_digest().to_owned();
        let commit = host.batch_commit(&proposal)?;
        debug_assert_eq!(commit.command_digest(), command_digest);
        debug_assert_eq!(commit.result_digest(), result_digest);

        let applied_count = end - self.completed_count;
        self.completed_count = end;
        let batch_index = self.completed_batches;
        self.completed_batches += 1;
        let after = BatchDocumentStamp::from_host(host);
        self.expected = after.clone();

        Ok(Some(OccurrenceBatchReceipt {
            schema: BATCH_RECEIPT_SCHEMA,
            workset_handle: self.workset_handle.clone(),
            scope: "occurrences",
            operation: self.operation.name(),
            operation_payload: self.operation,
            batch_index,
            applied_count,
            total_count: self.occurrence_ids.len(),
            completed_count: self.completed_count,
            remaining_count: self.occurrence_ids.len() - self.completed_count,
            complete: self.is_complete(),
            before,
            after,
            command_digest,
            result_digest,
            verified_write_count: commit.verified_writes().len(),
        }))
    }
}
