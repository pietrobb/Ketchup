mod harness;

use harness::Shell;
use ketchup_application::{DocumentSession, SessionSettings};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const OPEN_BUDGET: Duration = Duration::from_secs(5);
const EXACT_REBUILD_BUDGET: Duration = Duration::from_secs(180);
const UI_FRAMES: usize = 20;
const UI_FRAME_BUDGET: Duration = Duration::from_secs(5);
const MEMORY_GROWTH_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MEMORY_PROBE_ALLOCATION_BYTES: u64 = 64 * 1024 * 1024;
const MEMORY_PROBE_MIN_OBSERVED_BYTES: u64 = MEMORY_PROBE_ALLOCATION_BYTES / 2;
const MEMORY_PROBE_ENV: &str = "KETCHUP_MEMORY_HIGH_WATER_PROBE";
const MEMORY_PROBE_READY_ENV: &str = "KETCHUP_MEMORY_HIGH_WATER_PROBE_READY";
const MEMORY_PROBE_RELEASE_ENV: &str = "KETCHUP_MEMORY_HIGH_WATER_PROBE_RELEASE";

#[derive(Clone, Copy)]
struct CorpusEntry {
    name: &'static str,
    relative_path: &'static str,
    exact_must_be_complete: bool,
}

const CORPUS: [CorpusEntry; 3] = [
    CorpusEntry {
        name: "garden-studio",
        relative_path: "examples/garden-studio.ketchup",
        exact_must_be_complete: true,
    },
    CorpusEntry {
        name: "imported-hardware-drawer",
        relative_path: "examples/hettich-quadro-v6-drawer.ketchup",
        exact_must_be_complete: true,
    },
    CorpusEntry {
        name: "grooved-beam-array",
        relative_path: "examples/grooved-beam-array.ketchup",
        exact_must_be_complete: false,
    },
];

#[derive(Serialize)]
struct FixtureMetrics {
    name: &'static str,
    file_bytes: u64,
    definitions: usize,
    features: usize,
    feature_kind_count: usize,
    occurrences: usize,
    cold_open_ms: f64,
    exact_rebuild_ms: f64,
    exact_producers: usize,
    exact_complete: bool,
    topology_complete: bool,
    offscreen_ui_open_ms: f64,
    offscreen_ui_20_frames_ms: f64,
    offscreen_ui_frame_p95_ms: f64,
    memory_high_water_bytes_after: u64,
}

#[derive(Serialize)]
struct CorpusMetrics {
    schema: &'static str,
    build_profile: &'static str,
    claim_scope: &'static str,
    fixtures: Vec<FixtureMetrics>,
    total_definitions: usize,
    total_features: usize,
    total_occurrences: usize,
    memory_high_water_bytes_before: u64,
    memory_high_water_bytes_after: u64,
    memory_high_water_growth_bytes: u64,
    limitations: [&'static str; 5],
}

fn workspace_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

#[cfg(windows)]
fn process_memory_high_water_bytes() -> u64 {
    let command = format!(
        "[Console]::Write((Get-Process -Id {}).PeakPagedMemorySize64)",
        std::process::id()
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output()
        .expect("PowerShell must measure memory high-water");
    assert!(output.status.success(), "memory high-water query failed");
    String::from_utf8(output.stdout)
        .expect("memory high-water must be UTF-8 digits")
        .trim()
        .parse()
        .expect("memory high-water must be numeric")
}

#[cfg(windows)]
fn process_tree_memory_high_water_bytes() -> u64 {
    let command = format!(
        r#"$ErrorActionPreference = 'Stop'
$root = [uint32]{}
$self = [uint32]$PID
$processes = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId)
$pending = [System.Collections.ArrayList]::new()
[void]$pending.Add($root)
$ids = [System.Collections.ArrayList]::new()
while ($pending.Count -gt 0) {{
    $id = [uint32]$pending[0]
    $pending.RemoveAt(0)
    if ($id -eq $self) {{ continue }}
    [void]$ids.Add($id)
    foreach ($child in $processes) {{
        if ([uint32]$child.ParentProcessId -eq $id) {{
            [void]$pending.Add([uint32]$child.ProcessId)
        }}
    }}
}}
[uint64]$sum = 0
foreach ($id in $ids) {{
    $process = Get-Process -Id $id -ErrorAction Stop
    $sum = $sum + [uint64]$process.PeakPagedMemorySize64
}}
[Console]::Write($sum)"#,
        std::process::id()
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output()
        .expect("PowerShell must measure process-tree memory high-water");
    assert!(
        output.status.success(),
        "process-tree memory high-water query failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("process-tree memory high-water must be UTF-8 digits")
        .trim()
        .parse()
        .expect("process-tree memory high-water must be numeric")
}

#[cfg(target_os = "linux")]
fn linux_process_memory_high_water_bytes(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/status: {error}"));
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("VmHWM must be present for process {pid}"));
    kib * 1024
}

#[cfg(target_os = "linux")]
fn process_memory_high_water_bytes() -> u64 {
    linux_process_memory_high_water_bytes(std::process::id())
}

#[cfg(target_os = "linux")]
fn process_tree_memory_high_water_bytes() -> u64 {
    let root = std::process::id();
    let mut relationships = Vec::new();
    for entry in std::fs::read_dir("/proc").expect("enumerate /proc for process-tree memory") {
        let entry = entry.expect("read /proc entry");
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let parent = stat
            .rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_else(|| panic!("parse parent PID from /proc/{pid}/stat"));
        relationships.push((pid, parent));
    }

    let mut descendants = HashSet::from([root]);
    loop {
        let previous_len = descendants.len();
        for &(pid, parent) in &relationships {
            if descendants.contains(&parent) {
                descendants.insert(pid);
            }
        }
        if descendants.len() == previous_len {
            break;
        }
    }
    descendants.into_iter().fold(0_u64, |total, pid| {
        total
            .checked_add(linux_process_memory_high_water_bytes(pid))
            .expect("process-tree memory high-water overflow")
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_memory_high_water_bytes() -> u64 {
    panic!("memory high-water measurement is unsupported on this platform")
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_tree_memory_high_water_bytes() -> u64 {
    panic!("process-tree memory high-water measurement is unsupported on this platform")
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn p95(samples: &[Duration]) -> Duration {
    let mut ordered = samples.to_vec();
    ordered.sort_unstable();
    ordered[(ordered.len() * 95).div_ceil(100) - 1]
}

#[test]
fn memory_high_water_probe_child() {
    if std::env::var_os(MEMORY_PROBE_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }

    let ready_path =
        PathBuf::from(std::env::var_os(MEMORY_PROBE_READY_ENV).expect("memory probe ready path"));
    let release_path = PathBuf::from(
        std::env::var_os(MEMORY_PROBE_RELEASE_ENV).expect("memory probe release path"),
    );
    let high_water_before = process_memory_high_water_bytes();
    let mut allocation = vec![0_u8; usize::try_from(MEMORY_PROBE_ALLOCATION_BYTES).unwrap()];
    for page in allocation.chunks_mut(4096) {
        page[0] = 1;
    }
    std::hint::black_box(&allocation);
    let high_water_while_live = process_memory_high_water_bytes();
    assert!(
        high_water_while_live >= high_water_before.saturating_add(MEMORY_PROBE_MIN_OBSERVED_BYTES),
        "child memory high-water missed a live {MEMORY_PROBE_ALLOCATION_BYTES}-byte allocation"
    );
    std::fs::write(&ready_path, b"ready").expect("publish memory probe readiness");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !release_path.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(release_path.is_file(), "memory probe release timed out");
    std::hint::black_box(&allocation);
}

fn verify_memory_high_water_observes_child_allocation() {
    let directory = tempfile::tempdir().expect("memory probe directory");
    let ready_path = directory.path().join("ready");
    let release_path = directory.path().join("release");
    let high_water_before = process_tree_memory_high_water_bytes();
    let child = Command::new(std::env::current_exe().expect("test executable path"))
        .args(["--exact", "memory_high_water_probe_child", "--nocapture"])
        .env(MEMORY_PROBE_ENV, "1")
        .env(MEMORY_PROBE_READY_ENV, &ready_path)
        .env(MEMORY_PROBE_RELEASE_ENV, &release_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn isolated child memory high-water probe");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready_path.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let ready = ready_path.is_file();
    let high_water_with_child = process_tree_memory_high_water_bytes();
    std::fs::write(&release_path, b"release").expect("release child memory probe");
    let output = child
        .wait_with_output()
        .expect("wait for child memory high-water probe");
    assert!(
        ready && output.status.success(),
        "isolated child memory high-water probe failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        high_water_with_child >= high_water_before.saturating_add(MEMORY_PROBE_MIN_OBSERVED_BYTES),
        "process-tree memory high-water missed the child allocation: before={high_water_before}, with_child={high_water_with_child}"
    );
}

#[test]
fn process_tree_memory_high_water_observes_child_allocation() {
    verify_memory_high_water_observes_child_allocation();
}

#[test]
fn realistic_heterogeneous_corpus_measures_open_exact_ui_and_memory_without_parity_claims() {
    let worker = exact_worker_path();
    assert!(
        worker.is_file(),
        "missing exact worker: {}",
        worker.display()
    );
    verify_memory_high_water_observes_child_allocation();
    let memory_high_water_bytes_before = process_tree_memory_high_water_bytes();
    let mut memory_high_water_bytes_after = memory_high_water_bytes_before;
    let mut fixtures = Vec::with_capacity(CORPUS.len());

    for entry in CORPUS {
        let path = workspace_path(entry.relative_path);
        let file_bytes = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("{} is missing: {error}", path.display()))
            .len();

        let open_started = Instant::now();
        let mut session = DocumentSession::open(
            &path,
            SessionSettings {
                exact_worker_path: Some(worker.clone()),
                evaluation_timeout: EXACT_REBUILD_BUDGET,
            },
        )
        .unwrap_or_else(|error| panic!("{} cold open failed: {error}", entry.name));
        let cold_open = open_started.elapsed();
        assert!(
            cold_open < OPEN_BUDGET,
            "{} cold open took {cold_open:?}",
            entry.name
        );
        let snapshot = session.snapshot();
        let definitions = snapshot.definitions().count();
        let features = snapshot.features().count();
        let occurrences = snapshot.occurrences().count();
        let feature_kind_count = snapshot
            .features()
            .map(|feature| std::mem::discriminant(feature.kind()))
            .collect::<HashSet<_>>()
            .len();
        let identity = (
            snapshot.document_id(),
            snapshot.revision_id(),
            snapshot.canonical_digest(),
        );

        let rebuild_started = Instant::now();
        let report = session
            .evaluate()
            .unwrap_or_else(|error| panic!("{} exact rebuild failed: {error}", entry.name));
        let exact_rebuild = rebuild_started.elapsed();
        assert!(
            exact_rebuild < EXACT_REBUILD_BUDGET,
            "{} exact rebuild took {exact_rebuild:?}",
            entry.name
        );
        assert!(
            !report.producers.is_empty(),
            "{} has no exact producers",
            entry.name
        );
        if entry.exact_must_be_complete {
            assert!(
                report.complete,
                "{} exact render incomplete: {report:?}",
                entry.name
            );
            assert!(
                report.topology_complete,
                "{} exact topology incomplete: {report:?}",
                entry.name
            );
        }
        let after_rebuild = session.snapshot();
        assert_eq!(
            (
                after_rebuild.document_id(),
                after_rebuild.revision_id(),
                after_rebuild.canonical_digest(),
            ),
            identity,
            "{} evaluation mutated canonical state",
            entry.name
        );
        let exact_memory_high_water = process_tree_memory_high_water_bytes();
        drop(session);

        let mut shell = Shell::new();
        let ui_open_started = Instant::now();
        assert!(shell.app_mut().open_document_path(&path));
        shell.app_mut().enable_headless_instanced_scene();
        shell.step();
        let offscreen_ui_open = ui_open_started.elapsed();
        assert_eq!(shell.app().document_revision(), identity.1);
        assert_eq!(shell.app().canonical_digest(), identity.2);
        let mut frame_samples = Vec::with_capacity(UI_FRAMES);
        for _ in 0..UI_FRAMES {
            let started = Instant::now();
            shell.step();
            frame_samples.push(started.elapsed());
        }
        let ui_frames_total: Duration = frame_samples.iter().copied().sum();
        assert!(
            ui_frames_total < UI_FRAME_BUDGET,
            "{} twenty offscreen UI frames took {ui_frames_total:?}",
            entry.name
        );
        assert_eq!(shell.app().document_revision(), identity.1);
        assert_eq!(shell.app().canonical_digest(), identity.2);
        assert!(!shell.app().is_dirty());
        let ui_memory_high_water = process_tree_memory_high_water_bytes();
        drop(shell);

        let fixture_memory_high_water = exact_memory_high_water.max(ui_memory_high_water);
        memory_high_water_bytes_after =
            memory_high_water_bytes_after.max(fixture_memory_high_water);
        fixtures.push(FixtureMetrics {
            name: entry.name,
            file_bytes,
            definitions,
            features,
            feature_kind_count,
            occurrences,
            cold_open_ms: milliseconds(cold_open),
            exact_rebuild_ms: milliseconds(exact_rebuild),
            exact_producers: report.producers.len(),
            exact_complete: report.complete,
            topology_complete: report.topology_complete,
            offscreen_ui_open_ms: milliseconds(offscreen_ui_open),
            offscreen_ui_20_frames_ms: milliseconds(ui_frames_total),
            offscreen_ui_frame_p95_ms: milliseconds(p95(&frame_samples)),
            memory_high_water_bytes_after: fixture_memory_high_water,
        });
    }

    let total_definitions = fixtures.iter().map(|entry| entry.definitions).sum();
    let total_features = fixtures.iter().map(|entry| entry.features).sum();
    let total_occurrences = fixtures.iter().map(|entry| entry.occurrences).sum();
    assert!(
        total_definitions >= 134,
        "corpus is no longer definition-diverse: {total_definitions}"
    );
    assert!(
        total_features >= 348,
        "corpus is no longer feature-rich: {total_features}"
    );
    assert!(
        total_occurrences >= 590,
        "corpus is no longer assembly-scale: {total_occurrences}"
    );
    assert!(
        fixtures.iter().any(|entry| entry.feature_kind_count >= 3),
        "corpus lost multi-family feature coverage"
    );
    let memory_high_water_growth_bytes =
        memory_high_water_bytes_after.saturating_sub(memory_high_water_bytes_before);
    assert!(
        memory_high_water_growth_bytes < MEMORY_GROWTH_BUDGET_BYTES,
        "corpus memory high-water growth was {memory_high_water_growth_bytes} bytes"
    );

    let metrics = CorpusMetrics {
        schema: "ketchup.production-performance-corpus.v3",
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        claim_scope: "local Ketchup regression evidence only; no SolidWorks comparison measured",
        fixtures,
        total_definitions,
        total_features,
        total_occurrences,
        memory_high_water_bytes_before,
        memory_high_water_bytes_after,
        memory_high_water_growth_bytes,
        limitations: [
            "timings are wall-clock samples from one local run, not a cross-product benchmark",
            "UI samples are deterministic offscreen egui frames and do not claim native-window or hardware-GPU latency",
            "legacy documents with unsupported producers report exact incompleteness instead of being skipped",
            "memory is the live process-tree OS high-water sum: peak page-file usage on Windows and VmHWM on Linux",
            "regression budgets are generous failure ceilings, not advertised performance targets",
        ],
    };
    let json = serde_json::to_string_pretty(&metrics).unwrap();
    eprintln!("KETCHUP_PRODUCTION_PERFORMANCE={json}");
    if let Some(path) = std::env::var_os("KETCHUP_PRODUCTION_PERFORMANCE_PATH") {
        std::fs::write(path, format!("{json}\n")).expect("write requested performance evidence");
    }
}
