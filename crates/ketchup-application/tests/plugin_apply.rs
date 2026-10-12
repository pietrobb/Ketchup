//! A plugin edits the model with the typed CAD program MCP uses: the gateway
//! checks its capability, operation allowlist and budgets, and the program
//! then goes through the same planning and commit as MCP, one Undo step. What
//! the gateway refuses never reaches the document.

use ketchup_application::DocumentSession;
use ketchup_assistant::extension::{
    PluginCapability, PluginGateway, PluginGatewayError, PluginGrant, PluginLimits, PluginManifest,
    PluginRequest, PluginResponse,
};
use ketchup_assistant::sidecar::{AssistantCadEditOperation as Operation, AssistantCadEditProgram};
use ketchup_model::document::{FeatureId, FeatureParameterTarget, ParameterValueType, Snapshot};
use std::collections::BTreeSet;

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
        [PluginCapability::EditCadProgram],
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

/// What a host does with a plugin's `APPLY`: the gateway first, then the
/// MCP planning and commit.
fn apply(
    session: &mut DocumentSession,
    gateway: &mut PluginGateway,
    operations: Vec<Operation>,
) -> Result<(), PluginGatewayError> {
    let request = PluginRequest::Apply(AssistantCadEditProgram { operations });
    let PluginResponse::Program(program) = gateway.handle(&session.snapshot(), request)? else {
        panic!("APPLY answers with the program");
    };
    session
        .apply_cad_program(&program, &BTreeSet::new())
        .unwrap();
    Ok(())
}

/// Applies a program the gateway must refuse; the document stays as it was.
fn refused(
    session: &mut DocumentSession,
    gateway: &mut PluginGateway,
    operations: Vec<Operation>,
) -> PluginGatewayError {
    let digest = session.snapshot().canonical_digest();
    let undo = session.visible_undo_steps();
    let error = apply(session, gateway, operations).unwrap_err();
    assert_eq!(session.snapshot().canonical_digest(), digest);
    assert_eq!(session.visible_undo_steps(), undo);
    assert_eq!(height(session), 30.0);
    error
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
