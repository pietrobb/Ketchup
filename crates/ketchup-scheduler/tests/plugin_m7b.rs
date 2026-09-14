use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    ProposalCommitError, ProposalPrincipal,
};
use ketchup_core::extension::{PluginCapability, PluginGatewayError, PluginGrant, PluginLimits};
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: dimension("20", 20.0),
                },
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
    let baseline_bytes = ketchup_core::state_view::encode_semantic_state(&baseline.current())
        .agent_v1()
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
        ketchup_core::state_view::encode_semantic_state(&store.current())
            .agent_v1()
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
    let FeatureKind::Extrusion { height, .. } = snapshot.feature(EXTRUSION).unwrap().kind() else {
        panic!("fixture extrusion changed kind");
    };
    assert_eq!(height.millimetres(), 35.0);
}

#[test]
fn m7b_host_max_query_state_fits_the_declared_response_line() {
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
    let store = host_max_store();
    let script = "import sys,time\nprint('HELLO\\tketchup.plugin.v1\\torg.ketchup.backpressure\\t1.0.0\\t7001\\tquery.agent-state.v1\\t1\\t65536\\t1\\t1\\t1', flush=True)\nsys.stdin.readline()\nprint('QUERY\\tAGENT_STATE', flush=True)\ntime.sleep(5)";
    let (sender, receiver) = mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        let result = run_plugin_process(
            python(),
            &[OsString::from("-c"), OsString::from(script)],
            &store,
            pilot_grant(PluginLimits::HOST_MAX),
            Duration::from_millis(100),
            &AtomicBool::new(false),
        );
        let _ = sender.send(result);
    });

    let result = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("plugin host remained blocked writing a bounded response after its deadline");
    assert!(matches!(result, Err(PluginHostError::TimedOut)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn m7b_unrepresentable_timeout_is_rejected_without_panicking() {
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
    let store = host_max_store();
    let script = "import sys,time\nprint('HELLO\\tketchup.plugin.v1\\torg.ketchup.backpressure-cancel\\t1.0.0\\t7001\\tquery.agent-state.v1\\t1\\t65536\\t1\\t1\\t1', flush=True)\nsys.stdin.readline()\nprint('QUERY\\tAGENT_STATE', flush=True)\ntime.sleep(5)";
    let cancelled = Arc::new(AtomicBool::new(false));
    let run_cancelled = Arc::clone(&cancelled);
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = run_plugin_process(
            python(),
            &[OsString::from("-c"), OsString::from(script)],
            &store,
            pilot_grant(PluginLimits::HOST_MAX),
            Duration::from_secs(5),
            &run_cancelled,
        );
        let _ = sender.send(result);
    });
    std::thread::sleep(Duration::from_millis(100));
    cancelled.store(true, Ordering::Release);

    let result = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("plugin host remained blocked writing a bounded response after cancellation");
    assert!(matches!(result, Err(PluginHostError::Cancelled)));
}

#[test]
fn m7b_host_denies_ungranted_intent_and_request_or_query_budget_exhaustion() {
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

#[cfg(windows)]
#[test]
fn m7b_timeout_terminates_plugin_descendants() {
    let store = seed();
    let directory = tempfile::tempdir().unwrap();
    let sentinel = directory.path().join("escaped-descendant.txt");
    let script = "import subprocess,sys,time\nchild = 'import pathlib,sys,time; time.sleep(0.4); pathlib.Path(sys.argv[1]).write_text(\"escaped\")'\nsubprocess.Popen([sys.executable, '-c', child, sys.argv[1]])\nprint('HELLO\\tketchup.plugin.v1\\torg.ketchup.process-tree\\t1.0.0\\t7001\\t\\t1\\t1\\t1\\t1\\t1', flush=True)\ntime.sleep(30)";

    let result = run_plugin_process(
        python(),
        &[
            OsString::from("-c"),
            OsString::from(script),
            sentinel.as_os_str().to_owned(),
        ],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        Duration::from_millis(100),
        &AtomicBool::new(false),
    );

    assert!(matches!(result, Err(PluginHostError::TimedOut)));
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !sentinel.exists(),
        "plugin descendant survived host timeout and performed a delayed side effect"
    );
}

#[cfg(windows)]
#[test]
fn m7b_successful_run_terminates_plugin_descendants() {
    let store = seed();
    let directory = tempfile::tempdir().unwrap();
    let sentinel = directory.path().join("escaped-after-done.txt");
    let script = "import subprocess,sys,time\nchild = 'import pathlib,sys,time; time.sleep(0.4); pathlib.Path(sys.argv[1]).write_text(\"escaped\")'\nsubprocess.Popen([sys.executable, '-c', child, sys.argv[1]])\nprint('HELLO\tketchup.plugin.v1\torg.ketchup.process-tree\t1.0.0\t7001\t\t1\t1\t1\t1\t1', flush=True)\nsys.stdin.readline()\nprint('DONE', flush=True)\nsys.stdin.readline()";

    let run = run_plugin_process(
        python(),
        &[
            OsString::from("-c"),
            OsString::from(script),
            sentinel.as_os_str().to_owned(),
        ],
        &store,
        pilot_grant(PluginLimits::M7B_PILOT),
        Duration::from_secs(5),
        &AtomicBool::new(false),
    )
    .unwrap();

    assert_eq!(run.manifest.package(), "org.ketchup.process-tree");
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !sentinel.exists(),
        "plugin descendant survived a successful parent exit"
    );
}

#[test]
fn m7b_plugin_proposal_remains_revision_bound_and_non_replayable() {
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
