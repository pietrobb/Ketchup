mod harness;

use harness::Shell;
use ketchup_application::{DocumentSession, SessionSettings};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;
use std::time::{Duration, Instant};

const OPEN_BUDGET: Duration = Duration::from_secs(5);
const EXACT_REBUILD_BUDGET: Duration = Duration::from_secs(180);
const UI_FRAMES: usize = 20;
const UI_FRAME_BUDGET: Duration = Duration::from_secs(5);
const MEMORY_GROWTH_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy)]
struct CorpusEntry {
    name: &'static str,
    relative_path: &'static str,
    exact_must_be_complete: bool,
}

const CORPUS: [CorpusEntry; 5] = [
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
    CorpusEntry {
        name: "rounded-teapot",
        relative_path: "examples/assistant-rounded-teapot.ketchup",
        exact_must_be_complete: false,
    },
    CorpusEntry {
        name: "squeeze-bottle",
        relative_path: "examples/assistant-ketchup-squeeze-bottle.ketchup",
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
    private_bytes_after: u64,
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
    private_bytes_before: u64,
    peak_private_bytes: u64,
    private_bytes_growth: u64,
    limitations: [&'static str; 4],
}

fn workspace_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn exact_worker_path() -> PathBuf {
    std::env::current_exe()
        .expect("test executable path")
        .parent()
        .and_then(Path::parent)
        .expect("target profile directory")
        .join(if cfg!(windows) {
            "ketchup-exact-worker.exe"
        } else {
            "ketchup-exact-worker"
        })
}

#[cfg(windows)]
fn process_private_bytes() -> u64 {
    let command = format!(
        "[Console]::Write((Get-Process -Id {}).PrivateMemorySize64)",
        std::process::id()
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output()
        .expect("PowerShell must measure private bytes");
    assert!(output.status.success(), "private-byte query failed");
    String::from_utf8(output.stdout)
        .expect("private bytes must be UTF-8 digits")
        .trim()
        .parse()
        .expect("private bytes must be numeric")
}

#[cfg(target_os = "linux")]
fn process_private_bytes() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .expect("VmRSS must be present");
    kib * 1024
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_private_bytes() -> u64 {
    0
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
fn realistic_heterogeneous_corpus_measures_open_exact_ui_and_memory_without_parity_claims() {
    let worker = exact_worker_path();
    assert!(
        worker.is_file(),
        "missing exact worker: {}",
        worker.display()
    );
    let private_bytes_before = process_private_bytes();
    let mut peak_private_bytes = private_bytes_before;
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
        drop(shell);

        let private_bytes_after = process_private_bytes();
        peak_private_bytes = peak_private_bytes.max(private_bytes_after);
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
            private_bytes_after,
        });
    }

    let total_definitions = fixtures.iter().map(|entry| entry.definitions).sum();
    let total_features = fixtures.iter().map(|entry| entry.features).sum();
    let total_occurrences = fixtures.iter().map(|entry| entry.occurrences).sum();
    assert!(
        total_definitions >= 140,
        "corpus is no longer definition-diverse"
    );
    assert!(total_features >= 350, "corpus is no longer feature-rich");
    assert!(
        total_occurrences >= 590,
        "corpus is no longer assembly-scale"
    );
    assert!(
        fixtures.iter().any(|entry| entry.feature_kind_count >= 3),
        "corpus lost multi-family feature coverage"
    );
    let private_bytes_growth = peak_private_bytes.saturating_sub(private_bytes_before);
    assert!(
        private_bytes_growth < MEMORY_GROWTH_BUDGET_BYTES,
        "corpus private-byte growth was {private_bytes_growth} bytes"
    );

    let metrics = CorpusMetrics {
        schema: "ketchup.production-performance-corpus.v1",
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
        private_bytes_before,
        peak_private_bytes,
        private_bytes_growth,
        limitations: [
            "timings are wall-clock samples from one local run, not a cross-product benchmark",
            "UI samples are deterministic offscreen egui frames and do not claim native-window or hardware-GPU latency",
            "legacy documents with unsupported producers report exact incompleteness instead of being skipped",
            "regression budgets are generous failure ceilings, not advertised performance targets",
        ],
    };
    let json = serde_json::to_string_pretty(&metrics).unwrap();
    eprintln!("KETCHUP_PRODUCTION_PERFORMANCE={json}");
    if let Some(path) = std::env::var_os("KETCHUP_PRODUCTION_PERFORMANCE_PATH") {
        std::fs::write(path, format!("{json}\n")).expect("write requested performance evidence");
    }
}
