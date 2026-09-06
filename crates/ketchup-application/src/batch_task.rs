use crate::model_query::{ModelQuery, QueryError};
use crate::{DocumentSession, SessionError};
use ketchup_core::document::{CanonicalCommand, CommandBatch, OccurrenceId, ProposalBudget};
use serde::Serialize;

pub const MAX_OCCURRENCE_BATCH_ITEMS: usize = ProposalBudget::HOST_MAX.max_commands;
const BATCH_RECEIPT_SCHEMA: &str = "ketchup.occurrence-batch-receipt.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OccurrenceBatchOperation {
    SetColor { color: Option<[u8; 3]> },
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
    fn from_session(session: &DocumentSession) -> Self {
        let snapshot = session.snapshot();
        Self {
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            mutation_epoch: session.mutation_epoch(),
        }
    }
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

#[derive(Debug)]
pub enum OccurrenceBatchError {
    Query(QueryError),
    Session(SessionError),
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
            Self::Session(_) => "batch_transaction_failed",
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
        session: &DocumentSession,
        workset_handle: &str,
        operation: OccurrenceBatchOperation,
    ) -> Result<OccurrenceBatchTask, QueryError> {
        let snapshot = session.snapshot();
        Ok(OccurrenceBatchTask {
            workset_handle: workset_handle.to_owned(),
            operation,
            occurrence_ids: self.workset_occurrence_ids(&snapshot, workset_handle)?,
            completed_count: 0,
            completed_batches: 0,
            expected: BatchDocumentStamp::from_session(session),
            cancelled: false,
        })
    }
}

impl OccurrenceBatchTask {
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
        session: &mut DocumentSession,
    ) -> Result<Option<OccurrenceBatchReceipt>, OccurrenceBatchError> {
        self.commit_next_with_cancel(session, || false)
    }

    /// Cancellation linearizes at the second poll, immediately before the atomic commit.
    /// A request arriving after that poll applies to the next bounded transaction.
    pub fn commit_next_with_cancel(
        &mut self,
        session: &mut DocumentSession,
        mut cancellation_requested: impl FnMut() -> bool,
    ) -> Result<Option<OccurrenceBatchReceipt>, OccurrenceBatchError> {
        if self.is_complete() {
            return Ok(None);
        }
        if self.cancelled || cancellation_requested() {
            self.cancelled = true;
            return Err(OccurrenceBatchError::Cancelled);
        }

        let actual = BatchDocumentStamp::from_session(session);
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
        let proposal = session.plan_commands(CommandBatch::new(commands))?;

        if cancellation_requested() {
            self.cancelled = true;
            return Err(OccurrenceBatchError::Cancelled);
        }

        let before = self.expected.clone();
        let command_digest = proposal.command_digest().to_owned();
        let result_digest = proposal.intended_result_digest().to_owned();
        let commit = session.apply_proposal_verified(&proposal)?;
        debug_assert_eq!(commit.command_digest(), command_digest);
        debug_assert_eq!(commit.result_digest(), result_digest);

        let applied_count = end - self.completed_count;
        self.completed_count = end;
        let batch_index = self.completed_batches;
        self.completed_batches += 1;
        let after = BatchDocumentStamp::from_session(session);
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
