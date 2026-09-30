use ketchup_assistant::extension::{
    PluginCapability, PluginGatewayError, PluginGrant, PluginLimits,
};
use ketchup_geometry::sketch::{FeatureExtent, PadOperation, PadProfile, PadSpec};
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    ProposalCommitError, ProposalPrincipal,
};
use ketchup_scheduler::plugin::{PluginHostError, run_plugin_process};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

const DEFINITION: DefinitionId = DefinitionId(10);
const PROFILE: FeatureId = FeatureId(11);
const EXTRUSION: FeatureId = FeatureId(12);
const PRINCIPAL: u64 = 7001;

fn dimension(token: &str, value: f64) -> Dimension {
    Dimension::new(token, value).unwrap()
}

fn seed() -> DocumentStore {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::extrusion(PROFILE, dimension("20", 20.0)),
            },
        ]))
        .unwrap();
    store
}

fn python() -> OsString {
    std::env::var_os("PYTHON").unwrap_or_else(|| OsString::from("python"))
}

fn example_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("sdk")
        .join("python")
        .join("examples")
        .join("dimension_plugin.py")
}

fn pilot_grant(limits: PluginLimits) -> PluginGrant {
    PluginGrant::new(
        PRINCIPAL,
        [
            PluginCapability::QueryAgentState,
            PluginCapability::SetFeatureDimension,
        ],
        limits,
    )
}

fn host_max_store() -> DocumentStore {
    let mut baseline = DocumentStore::new();
    baseline
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "x".to_owned(),
            },
        ]))
        .unwrap();
    let baseline_bytes = ketchup_model::state_view::encode_semantic_state(&baseline.current())
        .agent()
        .len();
    let target_bytes = PluginLimits::HOST_MAX.max_query_bytes;
    assert!(baseline_bytes < target_bytes);
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "x".repeat(target_bytes - baseline_bytes + 1),
            },
        ]))
        .unwrap();
    assert_eq!(
        ketchup_model::state_view::encode_semantic_state(&store.current())
            .agent()
            .len(),
        target_bytes
    );
    store
}

fn run_example(
    store: &DocumentStore,
    grant: PluginGrant,
) -> Result<ketchup_scheduler::plugin::PluginRunResult, PluginHostError> {
    run_plugin_process(
        python(),
        &[
            example_script().into_os_string(),
            OsString::from(EXTRUSION.0.to_string()),
            OsString::from("35"),
        ],
        store,
        grant,
        Duration::from_secs(5),
        &AtomicBool::new(false),
    )
}

#[test]
fn m7b_python_plugin_queries_bounded_state_and_returns_one_review_only_proposal() {
    let _turn = crate::integration_support::file_turn();
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();

    let run = run_example(&store, pilot_grant(PluginLimits::M7B_PILOT)).unwrap();
    assert_eq!(run.manifest.package(), "org.ketchup.dimension-pilot");
    assert_eq!(run.manifest.principal_id(), PRINCIPAL);
    assert_eq!(run.query_count, 1);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    let proposal = run.proposal.unwrap();
    assert_eq!(proposal.principal(), ProposalPrincipal::Plugin(PRINCIPAL));
    assert_eq!(proposal.cost().commands, 1);
    assert_eq!(proposal.cost().write_targets, 1);
    store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    let snapshot = store.current();
    let FeatureKind::Pad(PadSpec {
        profile: PadProfile::Feature(_),
        extent: FeatureExtent::Blind(height),
        operation: PadOperation::NewBody,
        ..
    }) = snapshot.feature(EXTRUSION).unwrap().kind()
    else {
        panic!("fixture extrusion changed kind");
    };
    assert_eq!(height.millimetres(), 35.0);
}

#[test]
fn m7b_host_max_query_state_fits_the_declared_response_line() {
    let _turn = crate::integration_support::file_turn();
    let store = host_max_store();
    let script = "import sys\nprint('HELLO\\tketchup.plugin.v1\\torg.ketchup.host-max\\t1.0.0\\t7001\\tquery.agent-state.v1\\t2\\t65536\\t1\\t1\\t1', flush=True)\nsys.stdin.readline()\nprint('QUERY\\tAGENT_STATE', flush=True)\nstate = sys.stdin.readline()\nassert state.startswith('STATE\\t65536\\t')\nprint('DONE', flush=True)\nsys.stdin.readline()";

    let run = run_plugin_process(
        python(),
        &[OsString::from("-c"), OsString::from(script)],
        &store,
        pilot_grant(PluginLimits::HOST_MAX),
        Duration::from_secs(5),
        &AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(run.query_count, 1);
    assert!(run.proposal.is_none());
}

#[test]
fn m7b_host_max_response_honors_timeout_when_plugin_stops_reading() {
    let _turn = crate::integration_support::file_turn();
    let store = host_max_store();
    let directory = tempfile::tempdir().unwrap();
    let stopped_reading = directory.path().join("stopped-reading");
    let script = "import pathlib,sys,time\nprint('HELLO\\tketchup.plugin.v1\\torg.ketchup.backpressure\\t1.0.0\\t7001\\tquery.agent-state.v1\\t1\\t65536\\t1\\t1\\t1', flush=True)\nsys.stdin.readline()\nprint('QUERY\\tAGENT_STATE', flush=True)\npathlib.Path(sys.argv[1]).touch()\ntime.sleep(30)";
    let (sender, receiver) = mpsc::channel();
    let marker = stopped_reading.clone().into_os_string();
    std::thread::spawn(move || {
        let result = run_plugin_process(
            python(),
            &[OsString::from("-c"), OsString::from(script), marker],
            &store,
            pilot_grant(PluginLimits::HOST_MAX),
            // Long enough to reach the blocked response even with slow startup.
            Duration::from_secs(3),
            &AtomicBool::new(false),
        );
        let _ = sender.send(result);
    });

    // A host still blocked on the write would only return when the plugin
    // exits after 30 s; a responsive host returns shortly after its deadline.
    let result = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("plugin host remained blocked writing a bounded response after its deadline");
    assert!(matches!(result, Err(PluginHostError::TimedOut)));
    assert!(
        stopped_reading.exists(),
        "the deadline expired before the plugin blocked the host response"
    );
}

#[test]
fn m7b_flooding_plugin_is_backpressured_while_host_response_is_blocked() {
    let _turn = crate::integration_support::file_turn();
    let marker = std::env::temp_dir().join(format!(
        "ketchup-plugin-backpressure-{}.marker",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let script = "import pathlib,sys,time\nprint('HELLO\\tketchup.plugin.v1\\torg.ketchup.queue-backpressure\\t1.0.0\\t7001\\tquery.agent-state.v1\\t8\\t65536\\t1\\t1\\t1', flush=True)\nsys.stdin.readline()\nprint('QUERY\\tAGENT_STATE', flush=True)\nsys.stdout.write('QUERY\\tAGENT_STATE\\n' * 20000)\nsys.stdout.flush()\npathlib.Path(sys.argv[1]).touch()\ntime.sleep(5)";

    let result = run_plugin_process(
        python(),
        &[
            OsString::from("-c"),
            OsString::from(script),
            marker.clone().into_os_string(),
        ],
        &host_max_store(),
        pilot_grant(PluginLimits::HOST_MAX),
        Duration::from_millis(500),
        &AtomicBool::new(false),
    );

    assert!(
        matches!(result, Err(PluginHostError::TimedOut)),
        "unexpected plugin result: {result:?}"
    );
    assert!(
        !marker.exists(),
        "plugin stdout flood was drained into host memory instead of receiving backpressure"
    );
}

#[test]
fn m7b_unrepresentable_timeout_is_rejected_without_panicking() {
    let _turn = crate::integration_support::file_turn();
    let result = std::panic::catch_unwind(|| {
        run_plugin_process(
            python(),
            &[
                OsString::from("-c"),
                OsString::from("import time; time.sleep(5)"),
            ],
            &seed(),
            pilot_grant(PluginLimits::M7B_PILOT),
            Duration::MAX,
            &AtomicBool::new(false),
        )
    });

    assert!(
        matches!(result, Ok(Err(PluginHostError::InvalidTimeout))),
        "unrepresentable timeout must fail closed before plugin I/O"
    );
}

#[test]
fn m7b_host_max_response_honors_cancellation_when_plugin_stops_reading() {
    let _turn = crate::integration_support::file_turn();
    let store = host_max_store();
    let directory = tempfile::tempdir().unwrap();
    let stopped_reading = directory.path().join("stopped-reading");
    let script = "import pathlib,sys,time\nprint('HELLO\\tketchup.plugin.v1\\torg.ketchup.backpressure-cancel\\t1.0.0\\t7001\\tquery.agent-state.v1\\t1\\t65536\\t1\\t1\\t1', flush=True)\nsys.stdin.readline()\nprint('QUERY\\tAGENT_STATE', flush=True)\npathlib.Path(sys.argv[1]).touch()\ntime.sleep(30)";
    let cancelled = Arc::new(AtomicBool::new(false));
    let run_cancelled = Arc::clone(&cancelled);
    let (sender, receiver) = mpsc::channel();
    let marker = stopped_reading.clone().into_os_string();
    std::thread::spawn(move || {
        let result = run_plugin_process(
            python(),
            &[OsString::from("-c"), OsString::from(script), marker],
            &store,
            pilot_grant(PluginLimits::HOST_MAX),
            Duration::from_secs(60),
            &run_cancelled,
        );
        let _ = sender.send(result);
    });
    // Cancel only once the plugin has stopped reading, so the host is blocked
    // on its bounded response write rather than on process startup.
    let wait_started = Instant::now();
    while !stopped_reading.exists() {
        assert!(
            wait_started.elapsed() < Duration::from_secs(20),
            "plugin never reached the blocked response"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    cancelled.store(true, Ordering::Release);

    // A host still blocked on the write would only return when the plugin
    // exits after 30 s; a responsive host returns well within this bound.
    let result = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("plugin host remained blocked writing a bounded response after cancellation");
    assert!(matches!(result, Err(PluginHostError::Cancelled)));
}

#[test]
fn m7b_host_denies_ungranted_intent_and_request_or_query_budget_exhaustion() {
    let _turn = crate::integration_support::file_turn();
    let store = seed();
    let query_only = PluginGrant::new(
        PRINCIPAL,
        [PluginCapability::QueryAgentState],
        PluginLimits::M7B_PILOT,
    );
    assert!(matches!(
        run_example(&store, query_only),
        Err(PluginHostError::Gateway(
            PluginGatewayError::CapabilityDenied(PluginCapability::SetFeatureDimension)
        ))
    ));

    let one_request = PluginLimits {
        max_requests: 1,
        ..PluginLimits::M7B_PILOT
    };
    assert!(matches!(
        run_example(&store, pilot_grant(one_request)),
        Err(PluginHostError::Gateway(
            PluginGatewayError::RequestBudgetExceeded
        ))
    ));

    let one_query_byte = PluginLimits {
        max_query_bytes: 1,
        ..PluginLimits::M7B_PILOT
    };
    assert!(matches!(
        run_example(&store, pilot_grant(one_query_byte)),
        Err(PluginHostError::Gateway(
            PluginGatewayError::QueryBudgetExceeded { .. }
        ))
    ));
}

#[test]
fn m7b_plugin_process_does_not_inherit_parent_environment() {
    let _turn = crate::integration_support::file_turn();
    let store = seed();
    let script = "import os,sys\npackage = 'org.ketchup.ambient-leak' if os.environ.get('PATH') else 'org.ketchup.isolated'\nprint(f'HELLO\\tketchup.plugin.v1\\t{package}\\t1.0.0\\t7001\\t\\t1\\t1\\t1\\t1\\t1', flush=True)\nsys.stdin.readline()\nprint('DONE', flush=True)\nsys.stdin.readline()";

    let run = run_plugin_process(
        python(),
        &[OsString::from("-c"), OsString::from(script)],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        Duration::from_secs(5),
        &AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(run.manifest.package(), "org.ketchup.isolated");
}

#[test]
fn m7b_process_rejects_direct_mutation_vocabulary_and_oversized_input() {
    let _turn = crate::integration_support::file_turn();
    let store = seed();
    let hello = "HELLO\\tketchup.plugin.v1\\torg.ketchup.dimension-pilot\\t1.0.0\\t7001\\tquery.agent-state.v1,intent.set-feature-dimension.v1\\t4\\t32768\\t1\\t64\\t1";
    let direct_mutation = format!(
        "import sys; print('{hello}', flush=True); sys.stdin.readline(); print('MUTATE\\tRAW_DOCUMENT', flush=True); sys.stdin.readline()"
    );
    let result = run_plugin_process(
        python(),
        &[OsString::from("-c"), OsString::from(direct_mutation)],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        Duration::from_secs(5),
        &AtomicBool::new(false),
    );
    assert!(matches!(result, Err(PluginHostError::MalformedProtocol(_))));

    let oversized = "print('X' * 5000, flush=True)";
    let result = run_plugin_process(
        python(),
        &[OsString::from("-c"), OsString::from(oversized)],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        Duration::from_secs(5),
        &AtomicBool::new(false),
    );
    assert!(matches!(result, Err(PluginHostError::Transport(_))));
}

#[test]
fn m7b_pre_cancelled_run_does_not_attempt_to_spawn_the_plugin() {
    let _turn = crate::integration_support::file_turn();
    let store = seed();
    let cancelled = AtomicBool::new(true);
    let temp = tempfile::tempdir().unwrap();
    let missing_executable = temp.path().join("ketchup-plugin-must-not-spawn.exe");

    let result = run_plugin_process(
        missing_executable,
        &[],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        Duration::from_secs(5),
        &cancelled,
    );

    assert!(matches!(result, Err(PluginHostError::Cancelled)));
}

#[test]
fn m7b_process_timeout_and_cancellation_kill_the_untrusted_client() {
    let _turn = crate::integration_support::file_turn();
    let store = seed();
    let sleeper = "import time; time.sleep(5)";
    let result = run_plugin_process(
        python(),
        &[OsString::from("-c"), OsString::from(sleeper)],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        Duration::from_millis(50),
        &AtomicBool::new(false),
    );
    assert!(matches!(result, Err(PluginHostError::TimedOut)));

    let cancelled = AtomicBool::new(true);
    let result = run_plugin_process(
        python(),
        &[OsString::from("-c"), OsString::from(sleeper)],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        Duration::from_secs(5),
        &cancelled,
    );
    assert!(matches!(result, Err(PluginHostError::Cancelled)));
}

/// A plugin descendant whose side effect waits for the test's explicit
/// release, so the check does not race host latency under machine load.
#[cfg(windows)]
const DESCENDANT: &str = "import pathlib,sys,time\ns = sys.argv[1]\npathlib.Path(s + '.started').write_text('1')\nd = time.monotonic() + 30\nwhile not pathlib.Path(s + '.release').exists():\n    if time.monotonic() > d:\n        raise SystemExit(0)\n    time.sleep(0.01)\npathlib.Path(s).write_text('escaped')\n";

/// Spawns `DESCENDANT` (argv[2]) for sentinel argv[1] and waits until it runs.
#[cfg(windows)]
const SPAWN_DESCENDANT: &str = "import os,subprocess,sys,time\nsubprocess.Popen([sys.executable, '-c', sys.argv[2], sys.argv[1]])\nwhile not os.path.exists(sys.argv[1] + '.started'):\n    time.sleep(0.01)\n";

#[cfg(windows)]
fn release_descendant(sentinel: &std::path::Path) {
    let mut release = sentinel.as_os_str().to_owned();
    release.push(".release");
    std::fs::write(release, "1").unwrap();
    std::thread::sleep(Duration::from_secs(1));
}

#[cfg(windows)]
#[test]
fn m7b_timeout_terminates_plugin_descendants() {
    let _turn = crate::integration_support::file_turn();
    let store = seed();
    let directory = tempfile::tempdir().unwrap();
    let sentinel = directory.path().join("escaped-descendant.txt");
    let script = format!(
        "{SPAWN_DESCENDANT}print('HELLO\\tketchup.plugin.v1\\torg.ketchup.process-tree\\t1.0.0\\t7001\\t\\t1\\t1\\t1\\t1\\t1', flush=True)\ntime.sleep(30)"
    );

    let result = run_plugin_process(
        python(),
        &[
            OsString::from("-c"),
            OsString::from(script),
            sentinel.as_os_str().to_owned(),
            OsString::from(DESCENDANT),
        ],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        // Long enough for the plugin to start its descendant before the timeout.
        Duration::from_secs(3),
        &AtomicBool::new(false),
    );

    assert!(matches!(result, Err(PluginHostError::TimedOut)));
    release_descendant(&sentinel);
    assert!(
        !sentinel.exists(),
        "plugin descendant survived host timeout and performed a delayed side effect"
    );
}

#[cfg(windows)]
#[test]
fn m7b_completed_runs_terminate_plugin_descendants_on_success_and_failure() {
    let _turn = crate::integration_support::file_turn();
    for exit_code in [0, 7] {
        let store = seed();
        let directory = tempfile::tempdir().unwrap();
        let sentinel = directory.path().join("escaped-after-done.txt");
        let script = format!(
            "{SPAWN_DESCENDANT}print('HELLO\\tketchup.plugin.v1\\torg.ketchup.process-tree\\t1.0.0\\t7001\\t\\t1\\t1\\t1\\t1\\t1', flush=True)\nsys.stdin.readline()\nprint('DONE', flush=True)\nsys.stdin.readline()\nsys.exit(int(sys.argv[3]))"
        );

        let run = run_plugin_process(
            python(),
            &[
                OsString::from("-c"),
                OsString::from(script),
                sentinel.as_os_str().to_owned(),
                OsString::from(DESCENDANT),
                OsString::from(exit_code.to_string()),
            ],
            &store,
            pilot_grant(PluginLimits::M7B_PILOT),
            Duration::from_secs(5),
            &AtomicBool::new(false),
        )
        .map(|run| run.manifest.package().to_owned());

        if exit_code == 0 {
            assert_eq!(run.unwrap(), "org.ketchup.process-tree");
        } else {
            assert!(matches!(run, Err(PluginHostError::ExitedUnsuccessfully)));
        }
        release_descendant(&sentinel);
        assert!(
            !sentinel.exists(),
            "plugin descendant survived parent exit code {exit_code}"
        );
    }
}

#[test]
fn m7b_plugin_proposal_remains_revision_bound_and_non_replayable() {
    let _turn = crate::integration_support::file_turn();
    let mut store = seed();
    let run = run_example(&store, pilot_grant(PluginLimits::M7B_PILOT)).unwrap();
    let proposal = run.proposal.unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: dimension("30", 30.0),
            },
        ]))
        .unwrap();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
}
