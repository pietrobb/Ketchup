//! A plugin edits the model with the typed CAD program MCP uses: the gateway
//! checks its capability, operation allowlist and budgets, and the program
//! then goes through the same planning and commit as MCP, one Undo step. What
//! the gateway refuses, or what arrives after the document changed under the
//! plugin, never reaches the document.

use ketchup_application::{DocumentSession, PluginApplyError};
use ketchup_assistant::extension::{
    PluginCapability, PluginGateway, PluginGatewayError, PluginGrant, PluginLimits, PluginManifest,
    PluginRequest, PluginResponse,
};
use ketchup_assistant::sidecar::{AssistantCadEditOperation as Operation, AssistantCadEditProgram};
use ketchup_model::document::{FeatureId, FeatureParameterTarget, ParameterValueType, Snapshot};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const PRINCIPAL: u64 = 7001;

fn session() -> DocumentSession {
    let mut session = DocumentSession::default();
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![super::document_session::part()],
            },
            &BTreeSet::new(),
        )
        .unwrap();
    session
}

fn height_target(snapshot: &Snapshot) -> FeatureParameterTarget {
    snapshot
        .features()
        .map(|feature| {
            FeatureParameterTarget::new(feature.id(), "extent.distance", ParameterValueType::Length)
                .unwrap()
        })
        .find(|target| snapshot.feature_parameter_value(target).is_some())
        .unwrap()
}

fn height(session: &DocumentSession) -> f64 {
    let snapshot = session.snapshot();
    snapshot
        .feature_parameter_value(&height_target(&snapshot))
        .unwrap()
}

fn set_height(feature: FeatureId, value_mm: f64) -> Operation {
    Operation::SetDimension {
        feature_id: feature.0,
        constraint_id: None,
        value_mm,
    }
}

fn gateway(
    capabilities: &[PluginCapability],
    allowed: &[&str],
    limits: PluginLimits,
) -> PluginGateway {
    let manifest = PluginManifest::new(
        "org.example.heights",
        "1.0.0",
        PRINCIPAL,
        [
            PluginCapability::QueryAgentState,
            PluginCapability::EditCadProgram,
        ],
        PluginLimits::HOST_MAX,
    )
    .unwrap();
    let grant = PluginGrant::new(
        PRINCIPAL,
        capabilities.iter().copied(),
        allowed.iter().copied(),
        limits,
    );
    PluginGateway::new(manifest, grant).unwrap()
}

fn apply(
    session: &mut DocumentSession,
    gateway: &mut PluginGateway,
    operations: Vec<Operation>,
) -> Result<PluginResponse, PluginApplyError> {
    session.handle_plugin_request(
        gateway,
        PluginRequest::Apply(AssistantCadEditProgram { operations }),
    )
}

/// Applies a program that must be refused; the document and Undo stay as they were.
fn rejected(
    session: &mut DocumentSession,
    gateway: &mut PluginGateway,
    operations: Vec<Operation>,
) -> PluginApplyError {
    let digest = session.snapshot().canonical_digest();
    let undo = session.visible_undo_steps();
    let error = apply(session, gateway, operations).unwrap_err();
    assert_eq!(session.snapshot().canonical_digest(), digest);
    assert_eq!(session.visible_undo_steps(), undo);
    error
}

/// Applies a program the gateway must refuse.
fn refused(
    session: &mut DocumentSession,
    gateway: &mut PluginGateway,
    operations: Vec<Operation>,
) -> PluginGatewayError {
    let error = rejected(session, gateway, operations);
    assert_eq!(height(session), 30.0);
    match error {
        PluginApplyError::Gateway(error) => error,
        other => panic!("the gateway refuses, not {other}"),
    }
}

#[test]
fn a_granted_plugin_changes_the_model_in_one_undo_step() {
    let mut session = session();
    let feature = height_target(&session.snapshot()).feature_id;
    let before = session.snapshot().canonical_digest();
    let undo = session.visible_undo_steps();
    let mut plugin = gateway(
        &[PluginCapability::EditCadProgram],
        &["set_dimension"],
        PluginLimits::HOST_MAX,
    );

    apply(&mut session, &mut plugin, vec![set_height(feature, 45.0)]).unwrap();
    assert_eq!(height(&session), 45.0);
    assert_eq!(session.visible_undo_steps(), undo + 1);

    session.undo().unwrap();
    assert_eq!(height(&session), 30.0);
    assert_eq!(session.snapshot().canonical_digest(), before);
}

#[test]
fn a_plugin_without_the_edit_capability_is_refused() {
    let mut session = session();
    let feature = height_target(&session.snapshot()).feature_id;
    let mut plugin = gateway(
        &[PluginCapability::QueryAgentState],
        &["set_dimension"],
        PluginLimits::HOST_MAX,
    );
    assert_eq!(
        refused(&mut session, &mut plugin, vec![set_height(feature, 45.0)]),
        PluginGatewayError::CapabilityDenied(PluginCapability::EditCadProgram)
    );
}

#[test]
fn an_operation_outside_the_allowlist_refuses_the_whole_program() {
    let mut session = session();
    let feature = height_target(&session.snapshot()).feature_id;
    let mut plugin = gateway(
        &[PluginCapability::EditCadProgram],
        &["set_dimension"],
        PluginLimits::HOST_MAX,
    );
    assert_eq!(
        refused(
            &mut session,
            &mut plugin,
            vec![set_height(feature, 45.0), super::document_session::part()],
        ),
        PluginGatewayError::OperationNotAllowed("create_part".to_owned())
    );
}

#[test]
fn programs_and_requests_over_budget_are_refused() {
    let mut session = session();
    let feature = height_target(&session.snapshot()).feature_id;
    let one_each = PluginLimits {
        max_requests: 1,
        max_operations: 1,
        ..PluginLimits::HOST_MAX
    };
    let mut plugin = gateway(
        &[PluginCapability::EditCadProgram],
        &["set_dimension"],
        one_each,
    );
    assert_eq!(
        refused(
            &mut session,
            &mut plugin,
            vec![set_height(feature, 40.0), set_height(feature, 45.0)],
        ),
        PluginGatewayError::OperationBudgetExceeded {
            operations: 2,
            max_operations: 1,
        }
    );
    assert_eq!(
        refused(&mut session, &mut plugin, vec![set_height(feature, 45.0)]),
        PluginGatewayError::RequestBudgetExceeded
    );
}

#[test]
fn a_program_sent_after_the_document_changed_under_the_plugin_is_refused() {
    let mut session = session();
    let feature = height_target(&session.snapshot()).feature_id;
    let mut plugin = gateway(
        &[
            PluginCapability::QueryAgentState,
            PluginCapability::EditCadProgram,
        ],
        &["set_dimension"],
        PluginLimits::HOST_MAX,
    );
    let read = session.snapshot().revision_id();
    let response = session
        .handle_plugin_request(&mut plugin, PluginRequest::QueryAgentState)
        .unwrap();
    assert!(matches!(response, PluginResponse::AgentState(_)));

    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![set_height(feature, 40.0)],
            },
            &BTreeSet::new(),
        )
        .unwrap();
    let current = session.snapshot().revision_id();
    assert_ne!(current, read);

    let error = rejected(&mut session, &mut plugin, vec![set_height(feature, 45.0)]);
    assert!(
        matches!(
            error,
            PluginApplyError::Stale {
                read_revision,
                current_revision,
            } if read_revision == read && current_revision == current
        ),
        "{error}"
    );
    assert_eq!(height(&session), 40.0);
}

/// The example plugin from the Python SDK, run as a process: it reads the
/// state, sends one `set_dimension` and the host commits it as one Undo step.
#[test]
fn a_plugin_process_changes_the_model_in_one_undo_step() {
    let _turn = crate::integration_support::file_turn();
    let mut session = session();
    let feature = height_target(&session.snapshot()).feature_id;
    let before = session.snapshot().canonical_digest();
    let undo = session.visible_undo_steps();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sdk/python/examples/dimension_plugin.py");
    let grant = PluginGrant::new(
        PRINCIPAL,
        [
            PluginCapability::QueryAgentState,
            PluginCapability::EditCadProgram,
        ],
        ["set_dimension"],
        PluginLimits::M7B_PILOT,
    );

    let run = session
        .run_plugin(
            ketchup_test_env::python(),
            &[
                script.into_os_string(),
                OsString::from(feature.0.to_string()),
                OsString::from("35"),
            ],
            grant,
            Duration::from_secs(5),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(run.query_count, 1);
    assert_eq!(height(&session), 35.0);
    assert_eq!(session.visible_undo_steps(), undo + 1);

    session.undo().unwrap();
    assert_eq!(session.snapshot().canonical_digest(), before);
}
