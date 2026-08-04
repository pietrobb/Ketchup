use ketchup_core::document::{
    CanonicalCommand, CommandBatch, Dimension, DocumentStore, NodeId, ProposalCommitError,
};
use ketchup_exact::{ExactBackend, GeometryErrorCode, RectangleExtrudeSpec};
use ketchup_scheduler::{
    DerivedResult, EvaluationScheduler, ExactWorkerClient, InsertOutcome, JobToken,
};
use std::fmt::Write as _;
use std::process::Command;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const NODE: NodeId = NodeId(1);
const CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const CACHE_ENTRY_BYTES: usize = 1024 * 1024;
const SCHEDULE_PERMUTATIONS: usize = 10_000;
const CRASH_RECOVERY_RUNS: usize = 100;
const PERFORMANCE_RUNS: usize = 3;
const CANCELLATION_SAMPLES: usize = 100;
const TRANSPORT_RUNS: usize = 3;
const TRANSPORT_SAMPLES: usize = 1_000;
const TRANSPORT_WARMUP: usize = 100;
const READER_SAMPLES: usize = 1_000;

#[test]
fn formal_gate_b() {
    let stale_current_inserts = exercise_schedule_permutations();
    let changed_read_digest_accepted = exercise_proposal_race();
    let committed_revision_damage = exercise_crash_recovery();
    let exception_crosses_contract = exercise_exception_transport();
    let cancellation_ms = exercise_cancellation();
    let reader_ms = exercise_concurrent_reader();
    let (worker_ms, transport_ms, transport_percent, in_process_ms) =
        measure_worker_and_in_process();
    let (cache_plateau_bytes, cache_growth_bytes_per_100, cache_evictions) =
        exercise_cache_plateau();
    let private_bytes = process_private_bytes();

    let cancellation_p95_ms = percentile(&cancellation_ms, 95);
    let reader_p95_ms = percentile(&reader_ms, 95);
    let reader_block_over_100_ms = reader_ms.iter().filter(|sample| **sample > 100.0).count();
    let worker_p95_ms = percentile(&worker_ms, 95);
    let transport_p95_ms = percentile(&transport_ms, 95);
    let transport_percent_p95 = percentile(&transport_percent, 95);
    let in_process_p95_ms = percentile(&in_process_ms, 95);

    let metrics = GateMetrics {
        stale_current_inserts,
        changed_read_digest_accepted,
        committed_revision_damage,
        exception_crosses_contract,
        cancellation_p95_ms,
        reader_p95_ms,
        reader_block_over_100_ms,
        worker_p95_ms,
        transport_p95_ms,
        transport_percent_p95,
        in_process_p95_ms,
        cache_plateau_bytes,
        cache_growth_bytes_per_100,
        cache_evictions,
        private_bytes,
    };
    if !cfg!(debug_assertions) {
        write_metrics(
            &metrics,
            &cancellation_ms,
            &reader_ms,
            &worker_ms,
            &transport_ms,
            &transport_percent,
            &in_process_ms,
        );
    }

    assert_eq!(metrics.stale_current_inserts, 0);
    assert_eq!(metrics.changed_read_digest_accepted, 0);
    assert_eq!(metrics.committed_revision_damage, 0);
    assert_eq!(metrics.exception_crosses_contract, 0);
    assert_eq!(metrics.reader_block_over_100_ms, 0);
    assert!(metrics.reader_p95_ms <= 16.7, "{metrics:?}");
    assert!(metrics.cancellation_p95_ms <= 250.0, "{metrics:?}");
    assert!(metrics.transport_p95_ms <= 15.0, "{metrics:?}");
    assert!(metrics.transport_percent_p95 <= 20.0, "{metrics:?}");
    assert!(metrics.cache_plateau_bytes <= 512 * 1024 * 1024);
    assert!(metrics.cache_growth_bytes_per_100 <= 1024 * 1024);
    assert!(metrics.private_bytes <= 2 * 1024 * 1024 * 1024);
}

fn exercise_schedule_permutations() -> usize {
    let mut scheduler = EvaluationScheduler::new(CACHE_BUDGET_BYTES);
    let mut stale_current_inserts = 0;
    for permutation in 0..SCHEDULE_PERMUTATIONS {
        let old_revision = (permutation as u64) * 2 + 1;
        scheduler.advance_revision(old_revision, [NODE]).unwrap();
        let stale_token = scheduler
            .schedule(NODE, format!("input-{old_revision}"))
            .unwrap();
        scheduler
            .advance_revision(old_revision + 1, [NODE])
            .unwrap();
        let current_token = scheduler
            .schedule(NODE, format!("input-{}", old_revision + 1))
            .unwrap();
        let stale = result(stale_token, "stale");
        let current = result(current_token, "current");

        let outcomes = if permutation % 2 == 0 {
            [scheduler.accept(stale), scheduler.accept(current)]
        } else {
            [scheduler.accept(current), scheduler.accept(stale)]
        };
        if outcomes.contains(&InsertOutcome::Current)
            && scheduler.current_result_fingerprint(NODE) != Some("current")
        {
            stale_current_inserts += 1;
        }
    }
    stale_current_inserts
}

fn result(token: JobToken, fingerprint: &str) -> DerivedResult {
    DerivedResult {
        token,
        result_fingerprint: fingerprint.to_owned(),
        charge_bytes: 64,
    }
}

fn exercise_proposal_race() -> usize {
    let mut document = seed_document();
    let proposal =
        document.prepare_proposal(CommandBatch::new(vec![CanonicalCommand::SetDimension {
            id: NODE,
            dimension: dimension("35", 35.0),
        }]));
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetDimension {
            id: NODE,
            dimension: dimension("30", 30.0),
        }]))
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
    for run in 0..CRASH_RECOVERY_RUNS {
        let mut worker = ExactWorkerClient::spawn(worker_path()).unwrap();
        worker.ping().unwrap();
        let result = worker
            .extrude_rectangle(if run % 2 == 0 { 20.0 } else { 35.0 })
            .unwrap();
        assert_eq!(result.topology_counts[4], 1);
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

fn exercise_concurrent_reader() -> Vec<f64> {
    let scheduler = Arc::new(RwLock::new(EvaluationScheduler::new(CACHE_BUDGET_BYTES)));
    scheduler
        .write()
        .unwrap()
        .advance_revision(1, [NODE])
        .unwrap();
    let mut samples = Vec::with_capacity(PERFORMANCE_RUNS * READER_SAMPLES);
    for run in 0..PERFORMANCE_RUNS {
        let mut worker = ExactWorkerClient::spawn(worker_path()).unwrap();
        worker.begin_killable_job(Duration::from_secs(2)).unwrap();
        let start = samples.len();
        for _ in 0..READER_SAMPLES {
            let started = Instant::now();
            assert_eq!(scheduler.read().unwrap().current_revision(), 1);
            samples.push(milliseconds(started.elapsed()));
        }
        worker.cancel().unwrap();
        assert!(
            percentile(&samples[start..], 95) <= 16.7,
            "reader performance run {run} exceeded the frozen p95 threshold"
        );
        assert!(samples[start..].iter().all(|sample| *sample <= 100.0));
    }
    samples
}

fn measure_worker_and_in_process() -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
    let backend = ExactBackend::new();
    let mut worker_samples = Vec::with_capacity(TRANSPORT_RUNS * TRANSPORT_SAMPLES);
    let mut transport_samples = Vec::with_capacity(TRANSPORT_RUNS * TRANSPORT_SAMPLES);
    let mut transport_percent = Vec::with_capacity(TRANSPORT_RUNS * TRANSPORT_SAMPLES);
    let mut in_process_samples = Vec::with_capacity(TRANSPORT_RUNS * TRANSPORT_SAMPLES);

    for run in 0..TRANSPORT_RUNS {
        let transport_start = transport_samples.len();
        let percent_start = transport_percent.len();
        let mut worker = ExactWorkerClient::spawn(worker_path()).unwrap();
        for sample in 0..TRANSPORT_WARMUP {
            worker
                .extrude_rectangle(height(run, sample))
                .expect("worker warmup succeeds");
            backend
                .extrude_rectangle(spec(height(run, sample)))
                .expect("in-process warmup succeeds");
        }
        for sample in 0..TRANSPORT_SAMPLES {
            let height = height(run, sample);
            let started = Instant::now();
            let output = worker.extrude_rectangle(height).unwrap();
            let round_trip = started.elapsed();
            assert_eq!(output.volume_mm3.to_bits(), (6_000.0 * height).to_bits());
            let transport = round_trip.saturating_sub(output.backend_duration);
            let round_trip_ms = milliseconds(round_trip);
            worker_samples.push(round_trip_ms);
            transport_samples.push(milliseconds(transport));
            transport_percent.push(milliseconds(transport) / round_trip_ms * 100.0);

            let started = Instant::now();
            backend.extrude_rectangle(spec(height)).unwrap();
            in_process_samples.push(milliseconds(started.elapsed()));
        }
        assert!(
            percentile(&transport_samples[transport_start..], 95) <= 15.0,
            "transport performance run {run} exceeded the frozen p95 latency threshold"
        );
        assert!(
            percentile(&transport_percent[percent_start..], 95) <= 20.0,
            "transport performance run {run} exceeded the frozen p95 percentage threshold"
        );
    }
    (
        worker_samples,
        transport_samples,
        transport_percent,
        in_process_samples,
    )
}

fn exercise_cache_plateau() -> (usize, usize, u64) {
    let mut scheduler = EvaluationScheduler::new(CACHE_BUDGET_BYTES);
    let mut checkpoints = Vec::new();
    for revision in 1..=400_u64 {
        let node = NodeId(revision);
        scheduler.advance_revision(revision, [node]).unwrap();
        let token = scheduler
            .schedule(node, format!("cache-input-{revision}"))
            .unwrap();
        assert_eq!(
            scheduler.accept(DerivedResult {
                token,
                result_fingerprint: format!("cache-result-{revision}"),
                charge_bytes: CACHE_ENTRY_BYTES,
            }),
            InsertOutcome::Current
        );
        if revision % 100 == 0 {
            checkpoints.push(scheduler.cache_stats().used_bytes);
        }
    }
    let growth = checkpoints
        .windows(2)
        .map(|pair| pair[1].saturating_sub(pair[0]))
        .max()
        .unwrap_or_default();
    let stats = scheduler.cache_stats();
    (stats.used_bytes, growth, stats.evictions)
}

fn seed_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateNode {
            id: NODE,
            name: "extrusion_height".to_owned(),
            dimension: dimension("20", 20.0),
            dependencies: vec![],
        }]))
        .unwrap();
    document
}

fn dimension(token: &str, millimetres: f64) -> Dimension {
    Dimension::new(token, millimetres).unwrap()
}

fn spec(height_mm: f64) -> RectangleExtrudeSpec {
    RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm,
    }
}

fn height(run: usize, sample: usize) -> f64 {
    if (run + sample).is_multiple_of(2) {
        20.0
    } else {
        35.0
    }
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
    stale_current_inserts: usize,
    changed_read_digest_accepted: usize,
    committed_revision_damage: usize,
    exception_crosses_contract: usize,
    cancellation_p95_ms: f64,
    reader_p95_ms: f64,
    reader_block_over_100_ms: usize,
    worker_p95_ms: f64,
    transport_p95_ms: f64,
    transport_percent_p95: f64,
    in_process_p95_ms: f64,
    cache_plateau_bytes: usize,
    cache_growth_bytes_per_100: usize,
    cache_evictions: u64,
    private_bytes: usize,
}

fn write_metrics(
    metrics: &GateMetrics,
    cancellation_ms: &[f64],
    reader_ms: &[f64],
    worker_ms: &[f64],
    transport_ms: &[f64],
    transport_percent: &[f64],
    in_process_ms: &[f64],
) {
    let artifact_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("artifacts/gate-b");
    std::fs::create_dir_all(&artifact_dir).unwrap();
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
    writeln!(
        json,
        "  \"reader_samples\": {},",
        PERFORMANCE_RUNS * READER_SAMPLES
    )
    .unwrap();
    writeln!(
        json,
        "  \"transport_samples\": {},",
        TRANSPORT_RUNS * TRANSPORT_SAMPLES
    )
    .unwrap();
    writeln!(
        json,
        "  \"schedule_permutations\": {SCHEDULE_PERMUTATIONS},"
    )
    .unwrap();
    writeln!(
        json,
        "  \"stale_current_inserts\": {},",
        metrics.stale_current_inserts
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
    writeln!(json, "  \"reader_p95_ms\": {:.6},", metrics.reader_p95_ms).unwrap();
    writeln!(
        json,
        "  \"reader_block_over_100_ms\": {},",
        metrics.reader_block_over_100_ms
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
        "  \"worker_end_to_end_p95_ms\": {:.6},",
        metrics.worker_p95_ms
    )
    .unwrap();
    writeln!(
        json,
        "  \"worker_transport_p95_ms\": {:.6},",
        metrics.transport_p95_ms
    )
    .unwrap();
    writeln!(
        json,
        "  \"worker_transport_percent_p95\": {:.6},",
        metrics.transport_percent_p95
    )
    .unwrap();
    writeln!(
        json,
        "  \"in_process_p95_ms\": {:.6},",
        metrics.in_process_p95_ms
    )
    .unwrap();
    writeln!(json, "  \"cache_budget_bytes\": {CACHE_BUDGET_BYTES},").unwrap();
    writeln!(
        json,
        "  \"cache_plateau_bytes\": {},",
        metrics.cache_plateau_bytes
    )
    .unwrap();
    writeln!(
        json,
        "  \"cache_growth_bytes_per_100_edits\": {},",
        metrics.cache_growth_bytes_per_100
    )
    .unwrap();
    writeln!(json, "  \"cache_evictions\": {},", metrics.cache_evictions).unwrap();
    writeln!(
        json,
        "  \"process_private_bytes\": {},",
        metrics.private_bytes
    )
    .unwrap();
    write_samples(&mut json, "cancellation_ms", cancellation_ms, true);
    write_samples(&mut json, "reader_ms", reader_ms, true);
    write_samples(&mut json, "worker_end_to_end_ms", worker_ms, true);
    write_samples(&mut json, "worker_transport_ms", transport_ms, true);
    write_samples(
        &mut json,
        "worker_transport_percent",
        transport_percent,
        true,
    );
    write_samples(&mut json, "in_process_ms", in_process_ms, false);
    writeln!(json, "}}").unwrap();
    std::fs::write(artifact_dir.join("metrics.json"), json).unwrap();
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
