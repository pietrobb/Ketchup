use ketchup_exact::GeometryErrorCode;
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, Dimension, DocumentStore, NodeId, ProposalCommitError,
};
use ketchup_scheduler::protocol::{MAX_REQUEST_FRAME_BYTES, WorkerRequest, encode_frame};
use ketchup_scheduler::{ExactWorkerClient, WorkerError};
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

const NODE: NodeId = NodeId(1);
const CRASH_RECOVERY_RUNS: usize = 100;
const PERFORMANCE_RUNS: usize = 3;
const CANCELLATION_SAMPLES: usize = 100;

/// The pull-request suite checks only outcomes: how fast a cancellation is lives in
/// `perf_scheduler_cancellation` (review 2026-10-18 TST-5).
#[test]
fn scheduler_survives_races_crashes_and_cancellation() {
    let _turn = crate::integration_support::file_turn();
    assert_eq!(exercise_proposal_race(), 0);
    assert_eq!(exercise_crash_recovery(), 0);
    assert_eq!(exercise_exception_transport(), 0);
    cancel_running_jobs(3);
    assert!(process_private_bytes() <= 2 * 1024 * 1024 * 1024);
}

/// A running job is cancelled within 250 ms (p95) in each of three series of 100,
/// the frozen R0 threshold (docs/adr/0002-exact-backend-isolation.md).
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_scheduler_cancellation() {
    let _turn = crate::integration_support::file_turn();
    for run in 0..PERFORMANCE_RUNS {
        let p95 = percentile(&cancel_running_jobs(CANCELLATION_SAMPLES), 95);
        assert!(
            p95 <= 250.0,
            "cancellation series {run}: p95 {p95:.3} ms exceeds 250 ms"
        );
    }
}

/// A simulated crash ends the worker with a plain failure status. abort() would go
/// through Windows Error Reporting, which archives a report and holds the process
/// for seconds per crash, long enough to break the crash() deadline.
#[test]
fn simulated_worker_crash_ends_without_os_crash_reporting() {
    let mut worker = Command::new(worker_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let frame = encode_frame(&WorkerRequest::Crash, MAX_REQUEST_FRAME_BYTES).unwrap();
    worker.stdin.take().unwrap().write_all(&frame).unwrap();
    assert_eq!(worker.wait().unwrap().code(), Some(1));
}

/// The exact worker reads STEP and IGES files of any size: past half of this
/// computer's memory it is stopped with a named error, not left to exhaust the
/// system and the window with it (review 2026-10-21 LIM-3).
#[test]
fn the_exact_worker_is_stopped_past_its_memory_limit() {
    let _turn = crate::integration_support::file_turn();
    let limit = ketchup_scheduler::exact_worker_memory_bytes();
    let mut worker = ExactWorkerClient::spawn(worker_path()).unwrap();
    worker.allocate_probe(64 << 20).unwrap();
    let past_limit = u64::try_from(limit).unwrap() + (256 << 20);
    assert_eq!(
        worker.allocate_probe(past_limit),
        Err(WorkerError::MemoryLimit { bytes: limit })
    );
    let mut next = ExactWorkerClient::spawn(worker_path()).unwrap();
    next.ping().unwrap();
}

fn exercise_proposal_race() -> usize {
    let mut document = seed_document();
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NODE,
                dimension: dimension("35", 35.0),
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NODE,
                dimension: dimension("30", 30.0),
            },
        ]))
        .unwrap();
    usize::from(!matches!(
        document.commit_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ))
}

fn exercise_crash_recovery() -> usize {
    let document = seed_document();
    let committed_digest = document.current().canonical_digest();
    let mut damage = 0;
    for _ in 0..CRASH_RECOVERY_RUNS {
        let mut worker = ExactWorkerClient::spawn(worker_path()).unwrap();
        worker.ping().unwrap();
        worker.crash().unwrap();
        if document.current().canonical_digest() != committed_digest {
            damage += 1;
        }
    }
    damage
}

fn exercise_exception_transport() -> usize {
    let mut worker = ExactWorkerClient::spawn(worker_path()).unwrap();
    let mut crossed = 0;
    for _ in 0..100 {
        match worker.exception_probe() {
            Ok(code) if code == GeometryErrorCode::BackendException.as_str() => {}
            _ => crossed += 1,
        }
    }
    crossed
}

/// Cancels `count` running jobs, each leaving the committed revision as it was, and
/// returns how long each cancellation took in milliseconds.
fn cancel_running_jobs(count: usize) -> Vec<f64> {
    let document = seed_document();
    let committed_digest = document.current().canonical_digest();
    (0..count)
        .map(|_| {
            let mut worker = ExactWorkerClient::spawn(worker_path()).unwrap();
            worker.begin_killable_job(Duration::from_secs(2)).unwrap();
            std::thread::sleep(Duration::from_millis(2));
            let took = worker.cancel().unwrap();
            assert_eq!(document.current().canonical_digest(), committed_digest);
            milliseconds(took)
        })
        .collect()
}

fn seed_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NODE,
                name: "extrusion_height".to_owned(),
                dimension: dimension("20", 20.0),
                dependencies: vec![],
            },
        ]))
        .unwrap();
    document
}

fn dimension(token: &str, millimetres: f64) -> Dimension {
    Dimension::new(token, millimetres).unwrap()
}

fn worker_path() -> &'static str {
    env!("CARGO_BIN_EXE_ketchup-exact-worker")
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn percentile(samples: &[f64], percentile: usize) -> f64 {
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    let index = (ordered.len() * percentile).div_ceil(100) - 1;
    ordered[index]
}

fn process_private_bytes() -> usize {
    let command = format!(
        "[Console]::Write((Get-Process -Id {}).PrivateMemorySize64)",
        std::process::id()
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output()
        .expect("PowerShell measures private bytes on the Windows reference host");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}
