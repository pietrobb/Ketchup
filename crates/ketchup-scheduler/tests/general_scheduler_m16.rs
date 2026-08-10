use ketchup_core::document::{DerivedIdentity, NodeId, SlotPath, SlotSegment};
use ketchup_scheduler::AcceptanceIdentity;
use ketchup_scheduler::general::{
    CompletionOutcome, FailureOutcome, GeneralJobScheduler, JobError, JobFailureKind, JobKind,
    JobPolicy, JobProgress, JobRequest, JobStatus, ScheduleOutcome,
};

const CACHE_BUDGET: usize = 256;

fn identity(node_id: NodeId, key: &str) -> AcceptanceIdentity {
    let slot_path = SlotPath::new(vec![SlotSegment::new(node_id, "result", key).unwrap()]).unwrap();
    AcceptanceIdentity {
        document_scope: 42,
        derived_identity: DerivedIdentity::new(node_id, slot_path).unwrap(),
        input_digest: format!("input-{key}"),
        evaluator: format!("{key}-evaluator-v1"),
        backend: (key == "exact").then(|| "occt-v1".to_owned()),
        schema: format!("{key}-schema-v1"),
        tolerance: "model-v1".to_owned(),
    }
}

fn request(node: u64, kind: JobKind, key: &str, policy: JobPolicy) -> JobRequest {
    let node_id = NodeId(node);
    JobRequest {
        node_id,
        acceptance: identity(node_id, key),
        kind,
        policy,
    }
}

fn queued(outcome: ScheduleOutcome) -> ketchup_scheduler::general::JobHandle {
    match outcome {
        ScheduleOutcome::Queued(handle) => handle,
        ScheduleOutcome::CacheHit(evidence) => {
            panic!("expected cache miss, got {}", evidence.result_fingerprint)
        }
    }
}

#[test]
fn all_general_job_kinds_share_one_revision_safe_lifecycle_and_cache_contract() {
    let mut scheduler = GeneralJobScheduler::new(CACHE_BUDGET);
    scheduler.advance_revision(1, []).unwrap();
    let kinds = [
        JobKind::Exact,
        JobKind::Sketch,
        JobKind::Rule,
        JobKind::Mesh,
        JobKind::Validator,
    ];

    for (index, kind) in kinds.into_iter().enumerate() {
        let node = index as u64 + 1;
        let key = match kind {
            JobKind::Exact => "exact",
            JobKind::Sketch => "sketch",
            JobKind::Rule => "rule",
            JobKind::Mesh => "mesh",
            JobKind::Validator => "validator",
        };
        let handle = queued(
            scheduler
                .schedule(request(node, kind, key, JobPolicy::NO_RESTART))
                .unwrap(),
        );
        scheduler.start(handle.id).unwrap();
        scheduler
            .report_progress(
                handle.id,
                JobProgress {
                    completed_units: 1,
                    total_units: 2,
                },
            )
            .unwrap();
        scheduler
            .report_progress(
                handle.id,
                JobProgress {
                    completed_units: 2,
                    total_units: 2,
                },
            )
            .unwrap();
        assert_eq!(
            scheduler
                .complete(handle.id, format!("{key}-result"), 32)
                .unwrap(),
            CompletionOutcome::Current
        );

        let hit = scheduler
            .schedule(request(node, kind, key, JobPolicy::NO_RESTART))
            .unwrap();
        assert!(matches!(
            hit,
            ScheduleOutcome::CacheHit(evidence)
                if evidence.kind == kind
                    && evidence.result_fingerprint == format!("{key}-result")
                    && evidence.token.acceptance == identity(NodeId(node), key)
        ));
    }

    let telemetry = scheduler.telemetry();
    assert_eq!(telemetry.schedule_requests, 10);
    assert_eq!(telemetry.cache_misses, 5);
    assert_eq!(telemetry.cache_hits, 5);
    assert_eq!(telemetry.starts, 5);
    assert_eq!(telemetry.progress_updates, 10);
    assert_eq!(telemetry.completions, 5);
    assert_eq!(scheduler.active_job_count(), 0);
    assert_eq!(scheduler.cache_stats().entry_count, 5);
    assert_eq!(scheduler.cache_stats().used_bytes, 160);
}

#[test]
fn progress_is_monotonic_and_running_cancellation_requires_worker_acknowledgment() {
    let mut scheduler = GeneralJobScheduler::new(CACHE_BUDGET);
    scheduler.advance_revision(1, []).unwrap();
    let handle = queued(
        scheduler
            .schedule(request(1, JobKind::Mesh, "mesh", JobPolicy::NO_RESTART))
            .unwrap(),
    );
    scheduler.start(handle.id).unwrap();
    scheduler
        .report_progress(
            handle.id,
            JobProgress {
                completed_units: 4,
                total_units: 10,
            },
        )
        .unwrap();
    assert_eq!(
        scheduler.report_progress(
            handle.id,
            JobProgress {
                completed_units: 3,
                total_units: 10,
            }
        ),
        Err(JobError::ProgressRegression(handle.id))
    );
    assert_eq!(
        scheduler.report_progress(
            handle.id,
            JobProgress {
                completed_units: 5,
                total_units: 11,
            }
        ),
        Err(JobError::ProgressRegression(handle.id))
    );

    scheduler.request_cancel(handle.id).unwrap();
    assert!(scheduler.cancellation_requested(handle.id));
    assert!(matches!(
        scheduler.job(handle.id).unwrap().status,
        JobStatus::CancellationRequested {
            attempt: 1,
            progress: Some(JobProgress {
                completed_units: 4,
                total_units: 10
            })
        }
    ));
    assert_eq!(
        scheduler.complete(handle.id, "must-not-publish", 32),
        Err(JobError::NotRunning(handle.id))
    );
    scheduler.acknowledge_cancel(handle.id).unwrap();
    assert!(matches!(
        scheduler.job(handle.id).unwrap().status,
        JobStatus::Cancelled {
            attempts_started: 1
        }
    ));
    assert_eq!(scheduler.telemetry().cancellation_requests, 1);
    assert_eq!(scheduler.telemetry().cancellations, 1);
    assert_eq!(scheduler.cache_stats().entry_count, 0);
}

#[test]
fn retryable_failure_reschedules_once_then_fails_closed_at_the_policy_boundary() {
    let mut scheduler = GeneralJobScheduler::new(CACHE_BUDGET);
    scheduler.advance_revision(1, []).unwrap();
    let handle = queued(
        scheduler
            .schedule(request(1, JobKind::Exact, "exact", JobPolicy::ONE_RESTART))
            .unwrap(),
    );

    scheduler.start(handle.id).unwrap();
    assert_eq!(
        scheduler
            .fail(handle.id, JobFailureKind::Retryable)
            .unwrap(),
        FailureOutcome::RestartQueued
    );
    assert!(matches!(
        scheduler.job(handle.id).unwrap().status,
        JobStatus::Queued {
            attempts_started: 1
        }
    ));

    scheduler.start(handle.id).unwrap();
    assert_eq!(
        scheduler
            .fail(handle.id, JobFailureKind::Retryable)
            .unwrap(),
        FailureOutcome::Failed
    );
    assert!(matches!(
        scheduler.job(handle.id).unwrap().status,
        JobStatus::Failed { attempt: 2 }
    ));
    assert_eq!(scheduler.telemetry().starts, 2);
    assert_eq!(scheduler.telemetry().restarts, 1);
    assert_eq!(scheduler.telemetry().failures, 1);
    assert_eq!(scheduler.cache_stats().entry_count, 0);
}

#[test]
fn revision_change_marks_every_in_flight_job_stale_and_prevents_publication() {
    let mut scheduler = GeneralJobScheduler::new(CACHE_BUDGET);
    scheduler.advance_revision(1, []).unwrap();
    let running = queued(
        scheduler
            .schedule(request(1, JobKind::Rule, "rule", JobPolicy::NO_RESTART))
            .unwrap(),
    );
    let queued_job = queued(
        scheduler
            .schedule(request(
                2,
                JobKind::Validator,
                "validator",
                JobPolicy::NO_RESTART,
            ))
            .unwrap(),
    );
    scheduler.start(running.id).unwrap();

    scheduler.advance_revision(2, [NodeId(1)]).unwrap();

    for id in [running.id, queued_job.id] {
        assert!(matches!(
            scheduler.job(id).unwrap().status,
            JobStatus::Stale { .. }
        ));
    }
    assert_eq!(
        scheduler.complete(running.id, "late-result", 32),
        Err(JobError::NotRunning(running.id))
    );
    assert_eq!(scheduler.telemetry().stale_results, 2);
    assert_eq!(scheduler.active_job_count(), 0);
    assert_eq!(scheduler.cache_stats().entry_count, 0);
}

#[test]
fn terminal_failure_never_restarts_even_when_policy_allows_it() {
    let mut scheduler = GeneralJobScheduler::new(CACHE_BUDGET);
    scheduler.advance_revision(1, []).unwrap();
    let handle = queued(
        scheduler
            .schedule(request(
                1,
                JobKind::Sketch,
                "sketch",
                JobPolicy { max_restarts: 3 },
            ))
            .unwrap(),
    );
    scheduler.start(handle.id).unwrap();
    assert_eq!(
        scheduler.fail(handle.id, JobFailureKind::Terminal).unwrap(),
        FailureOutcome::Failed
    );
    assert_eq!(scheduler.telemetry().restarts, 0);
    assert_eq!(scheduler.telemetry().failures, 1);
}
