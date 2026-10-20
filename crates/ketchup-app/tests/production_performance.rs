use crate::harness;

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
const ONE_EDIT_BUDGET: Duration = Duration::from_millis(50);
const MEMORY_GROWTH_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MEMORY_PROBE_ALLOCATION_BYTES: u64 = 64 * 1024 * 1024;
const MEMORY_PROBE_MIN_OBSERVED_BYTES: u64 = MEMORY_PROBE_ALLOCATION_BYTES / 2;
/// Ends only a child that hangs before readiness; a slow child is waited for.
const MEMORY_PROBE_CHILD_HANG_GUARD: Duration = Duration::from_secs(300);
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

/// Total of [`process_tree_memory_high_waters`]; descendants of concurrent
/// tests are included, so the total can fall when one of them exits.
fn process_tree_memory_high_water_bytes() -> u64 {
    process_tree_memory_high_waters()
        .into_iter()
        .fold(0_u64, |total, (_, bytes)| {
            total
                .checked_add(bytes)
                .expect("process-tree memory high-water overflow")
        })
}

/// Memory high-water of this process and of every live descendant, by PID.
#[cfg(windows)]
fn process_tree_memory_high_waters() -> Vec<(u32, u64)> {
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
foreach ($id in $ids) {{
    # A descendant that exited after the listing no longer holds memory.
    $process = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($null -ne $process) {{
        [Console]::WriteLine("$id $([uint64]$process.PeakPagedMemorySize64)")
    }}
}}
# The exit code would otherwise be that of the last Get-Process, which fails
# quietly for a descendant that just exited.
exit 0"#,
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
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.trim()
                .split_once(' ')
                .and_then(|(pid, bytes)| Some((pid.parse().ok()?, bytes.parse().ok()?)))
                .unwrap_or_else(|| panic!("process-tree memory high-water line {line:?}"))
        })
        .collect()
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
fn process_tree_memory_high_waters() -> Vec<(u32, u64)> {
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
    descendants
        .into_iter()
        .map(|pid| (pid, linux_process_memory_high_water_bytes(pid)))
        .collect()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_memory_high_water_bytes() -> u64 {
    panic!("memory high-water measurement is unsupported on this platform")
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_tree_memory_high_waters() -> Vec<(u32, u64)> {
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
    // The parent compares the child's tree entry with this baseline; the
    // rename publishes it whole.
    let staged_path = ready_path.with_extension("staged");
    std::fs::write(&staged_path, high_water_before.to_string())
        .expect("stage memory probe baseline");
    std::fs::rename(&staged_path, &ready_path).expect("publish memory probe readiness");
    // The parent releases only after its process-tree query, which took over
    // 30 s under a full local suite; the guard only ends an orphaned child.
    let deadline = Instant::now() + MEMORY_PROBE_CHILD_HANG_GUARD;
    while !release_path.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(release_path.is_file(), "memory probe release timed out");
    std::hint::black_box(&allocation);
}

/// The process-tree walk must find a spawned child and read its allocation.
/// It compares the child's own entry with the baseline the child measured
/// before allocating: the tree total also holds descendants of concurrent
/// tests, and one of them exiting can lower it by more than the allocation.
fn verify_memory_high_water_observes_child_allocation() {
    let directory = tempfile::tempdir().expect("memory probe directory");
    let ready_path = directory.path().join("ready");
    let release_path = directory.path().join("release");
    let mut child = Command::new(std::env::current_exe().expect("test executable path"))
        .args([
            "--exact",
            &crate::integration_support::test_name(module_path!(), "memory_high_water_probe_child"),
            "--nocapture",
        ])
        .env(MEMORY_PROBE_ENV, "1")
        .env(MEMORY_PROBE_READY_ENV, &ready_path)
        .env(MEMORY_PROBE_RELEASE_ENV, &release_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn isolated child memory high-water probe");
    // Wait for the child's answer, ready or exited, not for a fixed time: on
    // a loaded runner the child took 13.5 s to get ready (CI run 39332521756).
    // The guard only ends a child that hangs, which then fails as not ready.
    let hang_guard = Instant::now() + MEMORY_PROBE_CHILD_HANG_GUARD;
    while !ready_path.is_file()
        && child.try_wait().expect("poll memory probe child").is_none()
        && Instant::now() < hang_guard
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let ready = ready_path.is_file();
    let hung = !ready && child.try_wait().expect("poll memory probe child").is_none();
    if hung {
        child.kill().expect("end hung memory probe child");
    }
    let child_id = child.id();
    let child_in_tree = process_tree_memory_high_waters()
        .into_iter()
        .find_map(|(pid, bytes)| (pid == child_id).then_some(bytes));
    std::fs::write(&release_path, b"release").expect("release child memory probe");
    let output = child
        .wait_with_output()
        .expect("wait for child memory high-water probe");
    assert!(
        ready && output.status.success(),
        "isolated child memory high-water probe failed (ready={ready}, hung={hung}):\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let child_before: u64 = std::fs::read_to_string(&ready_path)
        .expect("read memory probe baseline")
        .parse()
        .expect("memory probe baseline must be numeric");
    let child_in_tree = child_in_tree.unwrap_or_else(|| {
        panic!("process-tree memory high-water did not find child process {child_id}")
    });
    assert!(
        child_in_tree >= child_before.saturating_add(MEMORY_PROBE_MIN_OBSERVED_BYTES),
        "process-tree memory high-water missed the child allocation: child_before={child_before}, child_in_tree={child_in_tree}"
    );
}

#[test]
fn process_tree_memory_high_water_observes_child_allocation() {
    let _turn = crate::integration_support::file_turn();
    verify_memory_high_water_observes_child_allocation();
}

#[test]
fn realistic_heterogeneous_corpus_measures_open_exact_ui_and_memory_without_parity_claims() {
    corpus_metrics();
}

/// The time budgets of the corpus hold in release, nightly; the test above
/// checks what each step produced (review 2026-10-17 TST-3).
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_realistic_corpus_opens_rebuilds_and_draws_within_budget() {
    for fixture in corpus_metrics().fixtures {
        let name = fixture.name;
        let open = fixture.cold_open_ms;
        assert!(
            open < milliseconds(OPEN_BUDGET),
            "{name} cold open took {open} ms"
        );
        let rebuild = fixture.exact_rebuild_ms;
        assert!(
            rebuild < milliseconds(EXACT_REBUILD_BUDGET),
            "{name} exact rebuild took {rebuild} ms"
        );
        let frames = fixture.offscreen_ui_20_frames_ms;
        assert!(
            frames < milliseconds(UI_FRAME_BUDGET),
            "{name} twenty offscreen UI frames took {frames} ms"
        );
    }
}

/// Opens, rebuilds and draws each document of the corpus, checks what each
/// step produced and reports the times and memory.
fn corpus_metrics() -> CorpusMetrics {
    let _turn = crate::integration_support::file_turn();
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
    if let Some(path) = ketchup_test_env::report_path("production_performance.json") {
        std::fs::write(path, format!("{json}\n")).expect("write requested performance evidence");
    }
    metrics
}

#[test]
fn one_move_among_the_largest_indexed_model_commits() {
    fastest_move_among_the_largest_indexed_model();
}

/// One move in a model as large as the instance index serves commits within
/// [`ONE_EDIT_BUDGET`] in release: the edit validates what it changed, not the
/// whole model.
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_one_move_among_the_largest_indexed_model_commits_within_budget() {
    let fastest = fastest_move_among_the_largest_indexed_model();
    assert!(
        fastest < ONE_EDIT_BUDGET,
        "one move among the largest indexed model took {fastest:?}"
    );
}

/// The fastest of five moves of one occurrence among
/// `MAX_INSTANCE_INDEX_ITEMS`, each checked to have moved it.
fn fastest_move_among_the_largest_indexed_model() -> Duration {
    use ketchup_application::model_query::MAX_INSTANCE_INDEX_ITEMS;
    use ketchup_geometry::sketch::{
        FeatureDirection, FeatureExtent, PadOperation, PadProfile, PadSpec, PrincipalPlane,
        SketchEntity, SketchEntityId, SketchSpec, WorkplaneSpec,
    };
    use ketchup_model::document::{
        CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
        FeatureKind, OccurrenceId, Transform,
    };

    let definitions = 100_u64;
    let occurrences = MAX_INSTANCE_INDEX_ITEMS as u64;
    let mut commands = Vec::new();
    for definition in 1..=definitions {
        let base = definition * 10;
        let width = 400.0 + definition as f64;
        let corners = [[0.0, 0.0], [width, 0.0], [width, 200.0], [0.0, 200.0]];
        let sketch = SketchSpec {
            workplane: FeatureId(base),
            entities: (0..4)
                .map(|i| SketchEntity::Line {
                    id: SketchEntityId(i as u64 + 1),
                    start_mm: corners[i],
                    end_mm: corners[(i + 1) % 4],
                })
                .collect(),
            constraints: vec![],
        };
        let region = sketch.solved_regions().unwrap()[0].id;
        commands.push(CanonicalCommand::CreateDefinition {
            id: DefinitionId(definition),
            name: format!("part {definition}"),
        });
        for (id, kind) in [
            (
                base,
                FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            ),
            (base + 1, FeatureKind::Sketch(sketch)),
            (
                base + 2,
                FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: FeatureId(base + 1),
                        region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::new("18", 18.0).unwrap()),
                    operation: PadOperation::NewBody,
                }),
            ),
        ] {
            commands.push(CanonicalCommand::CreateFeature {
                id: FeatureId(id),
                definition_id: DefinitionId(definition),
                name: format!("feature {id}"),
                kind,
            });
        }
    }
    for id in 1..=occurrences {
        commands.push(CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(1 + id % definitions),
            name: format!("part {id}"),
            transform: Transform::from_translation(
                (id % 100) as f64 * 600.0,
                (id / 100) as f64 * 300.0,
                0.0,
            )
            .unwrap(),
            parent: None,
            tags: Default::default(),
            visible: true,
        });
    }
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    assert_eq!(
        document.current().occurrences().count(),
        MAX_INSTANCE_INDEX_ITEMS
    );

    // The fastest of several moves: a busy test machine only ever slows one down.
    (0..5_u32)
        .map(|step| {
            let id = OccurrenceId(occurrences / 2 + u64::from(step));
            let transform = Transform::from_translation(f64::from(step), 2.0, 3.0).unwrap();
            let move_one = CommandBatch::new(vec![CanonicalCommand::SetOccurrenceTransform {
                id,
                transform,
            }]);
            let started = Instant::now();
            document.apply_batch(&move_one).unwrap();
            let took = started.elapsed();
            assert_eq!(
                document.current().occurrence(id).map(|o| o.transform()),
                Some(transform)
            );
            took
        })
        .min()
        .unwrap()
}
