use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    ProposalCommitError, ProposalPrincipal,
};
use ketchup_core::extension::{PluginCapability, PluginGatewayError, PluginGrant, PluginLimits};
use ketchup_scheduler::plugin::{PluginHostError, run_plugin_process};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

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
