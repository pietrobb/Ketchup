use ketchup_exact::GeometryErrorCode;
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, Dimension, DocumentStore, NodeId, ProposalCommitError,
};
use ketchup_scheduler::ExactWorkerClient;
use ketchup_scheduler::protocol::{MAX_REQUEST_FRAME_BYTES, WorkerRequest, encode_frame};
use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::Duration;

const NODE: NodeId = NodeId(1);
const CRASH_RECOVERY_RUNS: usize = 100;
const PERFORMANCE_RUNS: usize = 3;
const CANCELLATION_SAMPLES: usize = 100;

#[test]
fn scheduler_survives_races_crashes_and_cancellation_within_limits() {
    let _turn = crate::integration_support::file_turn();
    let changed_read_digest_accepted = exercise_proposal_race();
    let committed_revision_damage = exercise_crash_recovery();
    let exception_crosses_contract = exercise_exception_transport();
    let cancellation_ms = exercise_cancellation();
    let private_bytes = process_private_bytes();

    let cancellation_p95_ms = percentile(&cancellation_ms, 95);

    let metrics = GateMetrics {
        changed_read_digest_accepted,
        committed_revision_damage,
        exception_crosses_contract,
        cancellation_p95_ms,
        private_bytes,
    };
    if !cfg!(debug_assertions) {
        write_metrics(&metrics, &cancellation_ms);
    }

    assert_eq!(metrics.changed_read_digest_accepted, 0);
    assert_eq!(metrics.committed_revision_damage, 0);
    assert_eq!(metrics.exception_crosses_contract, 0);
    assert!(metrics.cancellation_p95_ms <= 250.0, "{metrics:?}");
    assert!(metrics.private_bytes <= 2 * 1024 * 1024 * 1024);
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

fn exercise_cancellation() -> Vec<f64> {
    let document = seed_document();
    let committed_digest = document.current().canonical_digest();
    let mut samples = Vec::with_capacity(PERFORMANCE_RUNS * CANCELLATION_SAMPLES);
    for run in 0..PERFORMANCE_RUNS {
        let start = samples.len();
        for _ in 0..CANCELLATION_SAMPLES {
            let mut worker = ExactWorkerClient::spawn(worker_path()).unwrap();
            worker.begin_killable_job(Duration::from_secs(2)).unwrap();
            std::thread::sleep(Duration::from_millis(2));
            samples.push(milliseconds(worker.cancel().unwrap()));
            assert_eq!(document.current().canonical_digest(), committed_digest);
        }
        assert!(
            percentile(&samples[start..], 95) <= 250.0,
            "cancellation performance run {run} exceeded the frozen p95 threshold"
        );
    }
    samples
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

#[derive(Debug)]
struct GateMetrics {
    changed_read_digest_accepted: usize,
    committed_revision_damage: usize,
    exception_crosses_contract: usize,
    cancellation_p95_ms: f64,
    private_bytes: usize,
}

fn write_metrics(metrics: &GateMetrics, cancellation_ms: &[f64]) {
    let Some(metrics_path) = ketchup_test_env::report_path("scheduler_reliability.json") else {
        return;
    };
    let mut json = String::new();
    writeln!(json, "{{").unwrap();
    writeln!(json, "  \"gate\": \"B\",").unwrap();
    writeln!(json, "  \"decision_input\": \"measured\",").unwrap();
    writeln!(json, "  \"performance_runs\": {PERFORMANCE_RUNS},").unwrap();
    writeln!(
        json,
        "  \"cancellation_samples\": {},",
        PERFORMANCE_RUNS * CANCELLATION_SAMPLES
    )
    .unwrap();
    writeln!(json, "  \"crash_recovery_runs\": {CRASH_RECOVERY_RUNS},").unwrap();
    writeln!(
        json,
        "  \"committed_revision_damage\": {},",
        metrics.committed_revision_damage
    )
    .unwrap();
    writeln!(
        json,
        "  \"exception_crosses_contract\": {},",
        metrics.exception_crosses_contract
    )
    .unwrap();
    writeln!(
        json,
        "  \"changed_read_digest_accepted\": {},",
        metrics.changed_read_digest_accepted
    )
    .unwrap();
    writeln!(
        json,
        "  \"cancellation_p95_ms\": {:.6},",
        metrics.cancellation_p95_ms
    )
    .unwrap();
    writeln!(
        json,
        "  \"process_private_bytes\": {},",
        metrics.private_bytes
    )
    .unwrap();
    write_samples(&mut json, "cancellation_ms", cancellation_ms, false);
    writeln!(json, "}}").unwrap();
    std::fs::write(metrics_path, json).unwrap();
}

fn write_samples(json: &mut String, name: &str, samples: &[f64], comma: bool) {
    write!(json, "  \"{name}\": [").unwrap();
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        write!(json, "{sample:.6}").unwrap();
    }
    writeln!(json, "]{}", if comma { "," } else { "" }).unwrap();
}
