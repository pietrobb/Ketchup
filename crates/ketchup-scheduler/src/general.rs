use crate::{
    AcceptanceIdentity, CacheStats, DerivedResult, EvaluationScheduler, InsertOutcome, JobToken,
    SchedulerError,
};
use ketchup_core::document::NodeId;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum JobKind {
    Exact,
    Sketch,
    Rule,
    Mesh,
    Validator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobPolicy {
    pub max_restarts: u8,
}

impl JobPolicy {
    pub const NO_RESTART: Self = Self { max_restarts: 0 };
    pub const ONE_RESTART: Self = Self { max_restarts: 1 };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobRequest {
    pub node_id: NodeId,
    pub acceptance: AcceptanceIdentity,
    pub kind: JobKind,
    pub policy: JobPolicy,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct JobId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobHandle {
    pub id: JobId,
    pub token: JobToken,
    pub kind: JobKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheHitEvidence {
    pub token: JobToken,
    pub kind: JobKind,
    pub result_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleOutcome {
    CacheHit(CacheHitEvidence),
    Queued(JobHandle),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobProgress {
    pub completed_units: u64,
    pub total_units: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Queued {
        attempts_started: u8,
    },
    Running {
        attempt: u8,
        progress: Option<JobProgress>,
    },
    CancellationRequested {
        attempt: u8,
        progress: Option<JobProgress>,
    },
    Completed {
        attempt: u8,
        result_fingerprint: String,
    },
    Cancelled {
        attempts_started: u8,
    },
    Failed {
        attempt: u8,
    },
    Stale {
        attempts_started: u8,
    },
}

impl JobStatus {
    fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Queued { .. } | Self::Running { .. } | Self::CancellationRequested { .. }
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobSnapshot {
    pub handle: JobHandle,
    pub policy: JobPolicy,
    pub status: JobStatus,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct JobTelemetry {
    pub schedule_requests: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub starts: u64,
    pub progress_updates: u64,
    pub cancellation_requests: u64,
    pub cancellations: u64,
    pub restarts: u64,
    pub completions: u64,
    pub failures: u64,
    pub stale_results: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobFailureKind {
    Retryable,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureOutcome {
    RestartQueued,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionOutcome {
    Current,
    Stale,
}

#[derive(Clone, Debug)]
struct JobRecord {
    handle: JobHandle,
    policy: JobPolicy,
    status: JobStatus,
}

pub struct GeneralJobScheduler {
    scheduler: EvaluationScheduler,
    jobs: BTreeMap<JobId, JobRecord>,
    next_job_id: u64,
    telemetry: JobTelemetry,
}

impl GeneralJobScheduler {
    #[must_use]
    pub fn new(cache_budget_bytes: usize) -> Self {
        Self {
            scheduler: EvaluationScheduler::new(cache_budget_bytes),
            jobs: BTreeMap::new(),
            next_job_id: 1,
            telemetry: JobTelemetry::default(),
        }
    }

    #[must_use]
    pub const fn current_revision(&self) -> u64 {
        self.scheduler.current_revision()
    }

    pub fn advance_revision(
        &mut self,
        revision_id: u64,
        dirty_nodes: impl IntoIterator<Item = NodeId>,
    ) -> Result<(), JobError> {
        self.scheduler.advance_revision(revision_id, dirty_nodes)?;
        for record in self.jobs.values_mut() {
            if record.status.is_active() {
                record.status = JobStatus::Stale {
                    attempts_started: attempts_started(&record.status),
                };
                self.telemetry.stale_results += 1;
            }
        }
        Ok(())
    }

    pub fn schedule(&mut self, request: JobRequest) -> Result<ScheduleOutcome, JobError> {
        self.telemetry.schedule_requests += 1;
        let token = self
            .scheduler
            .schedule_with_identity(request.node_id, request.acceptance.clone())?;
        if let Some(result_fingerprint) = self
            .scheduler
            .current_result_fingerprint_for(request.node_id, &request.acceptance)
        {
            self.telemetry.cache_hits += 1;
            return Ok(ScheduleOutcome::CacheHit(CacheHitEvidence {
                token,
                kind: request.kind,
                result_fingerprint: result_fingerprint.to_owned(),
            }));
        }

        let id = JobId(self.next_job_id);
        self.next_job_id = self
            .next_job_id
            .checked_add(1)
            .ok_or(JobError::JobIdExhausted)?;
        let handle = JobHandle {
            id,
            token,
            kind: request.kind,
        };
        self.jobs.insert(
            id,
            JobRecord {
                handle: handle.clone(),
                policy: request.policy,
                status: JobStatus::Queued {
                    attempts_started: 0,
                },
            },
        );
        self.telemetry.cache_misses += 1;
        Ok(ScheduleOutcome::Queued(handle))
    }

    pub fn start(&mut self, id: JobId) -> Result<(), JobError> {
        let record = self.record_mut(id)?;
        let JobStatus::Queued { attempts_started } = record.status else {
            return Err(JobError::NotQueued(id));
        };
        let attempt = attempts_started
            .checked_add(1)
            .ok_or(JobError::AttemptLimitExceeded(id))?;
        record.status = JobStatus::Running {
            attempt,
            progress: None,
        };
        self.telemetry.starts += 1;
        Ok(())
    }

    pub fn report_progress(&mut self, id: JobId, progress: JobProgress) -> Result<(), JobError> {
        if progress.total_units == 0 || progress.completed_units > progress.total_units {
            return Err(JobError::InvalidProgress(id));
        }
        let record = self.record_mut(id)?;
        let JobStatus::Running {
            progress: previous, ..
        } = &mut record.status
        else {
            return Err(JobError::NotRunning(id));
        };
        if previous.as_ref().is_some_and(|previous| {
            progress.total_units != previous.total_units
                || progress.completed_units < previous.completed_units
        }) {
            return Err(JobError::ProgressRegression(id));
        }
        *previous = Some(progress);
        self.telemetry.progress_updates += 1;
        Ok(())
    }

    pub fn request_cancel(&mut self, id: JobId) -> Result<(), JobError> {
        let cancelled_immediately = {
            let record = self.record_mut(id)?;
            match &record.status {
                JobStatus::Queued { attempts_started } => {
                    record.status = JobStatus::Cancelled {
                        attempts_started: *attempts_started,
                    };
                    Some(true)
                }
                JobStatus::Running { attempt, progress } => {
                    record.status = JobStatus::CancellationRequested {
                        attempt: *attempt,
                        progress: *progress,
                    };
                    Some(false)
                }
                JobStatus::CancellationRequested { .. } => None,
                _ => return Err(JobError::NotCancellable(id)),
            }
        };
        if let Some(cancelled_immediately) = cancelled_immediately {
            self.telemetry.cancellation_requests += 1;
            self.telemetry.cancellations += u64::from(cancelled_immediately);
        }
        Ok(())
    }

    #[must_use]
    pub fn cancellation_requested(&self, id: JobId) -> bool {
        self.jobs
            .get(&id)
            .is_some_and(|record| matches!(record.status, JobStatus::CancellationRequested { .. }))
    }

    pub fn acknowledge_cancel(&mut self, id: JobId) -> Result<(), JobError> {
        let record = self.record_mut(id)?;
        let JobStatus::CancellationRequested { attempt, .. } = record.status else {
            return Err(JobError::CancellationNotRequested(id));
        };
        record.status = JobStatus::Cancelled {
            attempts_started: attempt,
        };
        self.telemetry.cancellations += 1;
        Ok(())
    }

    pub fn fail(&mut self, id: JobId, kind: JobFailureKind) -> Result<FailureOutcome, JobError> {
        let record = self.record_mut(id)?;
        let JobStatus::Running { attempt, .. } = record.status else {
            return Err(JobError::NotRunning(id));
        };
        if kind == JobFailureKind::Retryable && attempt <= record.policy.max_restarts {
            record.status = JobStatus::Queued {
                attempts_started: attempt,
            };
            self.telemetry.restarts += 1;
            Ok(FailureOutcome::RestartQueued)
        } else {
            record.status = JobStatus::Failed { attempt };
            self.telemetry.failures += 1;
            Ok(FailureOutcome::Failed)
        }
    }

    pub fn complete(
        &mut self,
        id: JobId,
        result_fingerprint: impl Into<String>,
        charge_bytes: usize,
    ) -> Result<CompletionOutcome, JobError> {
        let result_fingerprint = result_fingerprint.into();
        if result_fingerprint.is_empty() {
            return Err(JobError::EmptyResultFingerprint(id));
        }
        let (attempt, token) = {
            let record = self.record_mut(id)?;
            let JobStatus::Running { attempt, .. } = record.status else {
                return Err(JobError::NotRunning(id));
            };
            (attempt, record.handle.token.clone())
        };
        let outcome = self.scheduler.accept(DerivedResult {
            token,
            result_fingerprint: result_fingerprint.clone(),
            charge_bytes,
        });
        let record = self.record_mut(id)?;
        match outcome {
            InsertOutcome::Current => {
                record.status = JobStatus::Completed {
                    attempt,
                    result_fingerprint,
                };
                self.telemetry.completions += 1;
                Ok(CompletionOutcome::Current)
            }
            InsertOutcome::Stale => {
                record.status = JobStatus::Stale {
                    attempts_started: attempt,
                };
                self.telemetry.stale_results += 1;
                Ok(CompletionOutcome::Stale)
            }
        }
    }

    #[must_use]
    pub fn job(&self, id: JobId) -> Option<JobSnapshot> {
        self.jobs.get(&id).map(|record| JobSnapshot {
            handle: record.handle.clone(),
            policy: record.policy,
            status: record.status.clone(),
        })
    }

    #[must_use]
    pub fn telemetry(&self) -> JobTelemetry {
        self.telemetry
    }

    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        self.scheduler.cache_stats()
    }

    #[must_use]
    pub fn active_job_count(&self) -> usize {
        self.jobs
            .values()
            .filter(|record| record.status.is_active())
            .count()
    }

    fn record_mut(&mut self, id: JobId) -> Result<&mut JobRecord, JobError> {
        self.jobs.get_mut(&id).ok_or(JobError::UnknownJob(id))
    }
}

fn attempts_started(status: &JobStatus) -> u8 {
    match status {
        JobStatus::Queued { attempts_started }
        | JobStatus::Cancelled { attempts_started }
        | JobStatus::Stale { attempts_started } => *attempts_started,
        JobStatus::Running { attempt, .. }
        | JobStatus::CancellationRequested { attempt, .. }
        | JobStatus::Completed { attempt, .. }
        | JobStatus::Failed { attempt } => *attempt,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobError {
    Scheduler(SchedulerError),
    UnknownJob(JobId),
    NotQueued(JobId),
    NotRunning(JobId),
    NotCancellable(JobId),
    CancellationNotRequested(JobId),
    InvalidProgress(JobId),
    ProgressRegression(JobId),
    EmptyResultFingerprint(JobId),
    AttemptLimitExceeded(JobId),
    JobIdExhausted,
}

impl From<SchedulerError> for JobError {
    fn from(error: SchedulerError) -> Self {
        Self::Scheduler(error)
    }
}

impl fmt::Display for JobError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scheduler(error) => write!(formatter, "scheduler rejected job: {error}"),
            Self::UnknownJob(id) => write!(formatter, "job {} does not exist", id.0),
            Self::NotQueued(id) => write!(formatter, "job {} is not queued", id.0),
            Self::NotRunning(id) => write!(formatter, "job {} is not running", id.0),
            Self::NotCancellable(id) => write!(formatter, "job {} is not cancellable", id.0),
            Self::CancellationNotRequested(id) => {
                write!(formatter, "job {} has no cancellation request", id.0)
            }
            Self::InvalidProgress(id) => write!(formatter, "job {} progress is invalid", id.0),
            Self::ProgressRegression(id) => {
                write!(formatter, "job {} progress regressed", id.0)
            }
            Self::EmptyResultFingerprint(id) => {
                write!(formatter, "job {} result fingerprint is empty", id.0)
            }
            Self::AttemptLimitExceeded(id) => {
                write!(formatter, "job {} attempt counter overflowed", id.0)
            }
            Self::JobIdExhausted => formatter.write_str("job identifier space is exhausted"),
        }
    }
}

impl std::error::Error for JobError {}
