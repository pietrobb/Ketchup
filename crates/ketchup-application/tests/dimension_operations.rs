//! `upsert_dimension`, `delete_dimension` and `set_evaluator_input`: a
//! dimension an AI declares shows the live value of a feature parameter,
//! commits as one Undo step, survives save and reopen, disappears when
//! deleted and comes back with Undo; what does not resolve is refused before
//! anything changes.

use ketchup_application::model_query::{EntityKind, ModelQuery};
use ketchup_application::{DocumentSession, SaveOptions, SessionError, SessionSettings};
use ketchup_assistant::catalog::cad_operation_catalog;
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation as Operation, AssistantCadEditProgram,
    AssistantCadParameterValueType, AssistantDimensionUnit,
};
use ketchup_model::document::{
    FeatureId, FeatureParameterTarget, NodeId, ParameterValueType, Snapshot,
};
use serde_json::Value;
use std::collections::BTreeSet;

fn program(operations: Vec<Operation>) -> AssistantCadEditProgram {
    AssistantCadEditProgram { operations }
}

fn commit(session: &mut DocumentSession, operations: Vec<Operation>) {
    session
        .apply_cad_program(&program(operations), &BTreeSet::new())
        .unwrap();
}

fn height(dimension_id: u64, feature_id: FeatureId, path: &str) -> Operation {
    Operation::UpsertDimension {
        dimension_id,
        name: "Part height".to_owned(),
        feature_id: feature_id.0,
        parameter_path: path.to_owned(),
        unit: Some(AssistantDimensionUnit::Cm),
        decimal_places: Some(2),
    }
}

/// The extrusion of the 30 mm tall part from `document_session::part`.
fn extrusion(snapshot: &Snapshot) -> FeatureId {
    snapshot
        .features()
        .map(|feature| feature.id())
        .find(|id| {
            let target =
                FeatureParameterTarget::new(*id, "extent.distance", ParameterValueType::Length)
                    .unwrap();
            snapshot.feature_parameter_value(&target).is_some()
        })
        .unwrap()
}

/// The dimension as MCP `inspect query kind=dimensions` reports it.
fn reported(snapshot: &Snapshot, id: u64) -> Option<Value> {
    ModelQuery::default()
        .detail(snapshot, EntityKind::Dimensions, id)
        .ok()
        .map(|detail| detail["item"].clone())
}

#[test]
fn the_catalog_offers_dimension_operations() {
    let upsert = cad_operation_catalog(Some("upsert_dimension")).unwrap();
    let fields = upsert["fields"].as_str().unwrap();
    assert!(fields.contains("parameter_path"), "{fields}");
    assert!(fields.contains("unit?"), "{fields}");
    assert!(cad_operation_catalog(Some("delete_dimension")).is_ok());
    assert!(cad_operation_catalog(Some("set_evaluator_input")).is_ok());
}

#[test]
fn a_dimension_shows_its_value_survives_reopen_and_undo_restores_it_after_deletion() {
    let mut session = DocumentSession::default();
    commit(&mut session, vec![super::document_session::part()]);
    let feature = extrusion(&session.snapshot());
    let undo = session.visible_undo_steps();

    commit(&mut session, vec![height(90, feature, "extent.distance")]);
    assert_eq!(session.visible_undo_steps(), undo + 1);
    let created = session.snapshot();
    let row = reported(&created, 90).expect("the created dimension is reported");
    assert_eq!(row["name"]["text"], "Part height", "{row:#}");
    assert_eq!(row["status"], "resolved", "{row:#}");
    assert_eq!(row["value_mm"], 30.0, "{row:#}");
    assert_eq!(row["display_text"], "3.00 cm", "{row:#}");
    assert_eq!(row["target"]["feature_id"], feature.0);

    // The dimension follows the parameter it shows.
    commit(
        &mut session,
        vec![Operation::SetFeatureParameter {
            feature_id: feature.0,
            parameter_path: "extent.distance".into(),
            value_type: AssistantCadParameterValueType::Length,
            value: 45.0.into(),
        }],
    );
    assert_eq!(reported(&session.snapshot(), 90).unwrap()["value_mm"], 45.0);
    session.undo().unwrap();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dimensions.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reported(&reopened.snapshot(), 90), Some(row.clone()));

    commit(
        &mut session,
        vec![Operation::DeleteDimension { dimension_id: 90 }],
    );
    assert!(reported(&session.snapshot(), 90).is_none());
    assert_eq!(session.snapshot().persistent_dimensions().count(), 0);
    session.undo().unwrap();
    assert_eq!(reported(&session.snapshot(), 90), Some(row));
    assert_eq!(
        session.snapshot().canonical_digest(),
        created.canonical_digest()
    );
}

#[test]
fn an_evaluator_input_takes_a_new_value_in_one_undo_step() {
    let mut session = DocumentSession::default();
    commit(
        &mut session,
        vec![Operation::CreateEvaluatorInput {
            node_id: 7,
            name: "shelf_depth".to_owned(),
            value: 300.0,
        }],
    );
    let before = session.snapshot().canonical_digest();
    commit(
        &mut session,
        vec![Operation::SetEvaluatorInput {
            node_id: 7,
            value: 350.0,
        }],
    );
    let node = session
        .snapshot()
        .evaluator_node(NodeId(7))
        .cloned()
        .unwrap();
    assert_eq!(node.name(), "shelf_depth");
    assert_eq!(node.dimension().unwrap().millimetres(), 350.0);
    session.undo().unwrap();
    assert_eq!(session.snapshot().canonical_digest(), before);
}

fn set_length(feature: FeatureId, path: &str, value: f64) -> Operation {
    Operation::SetFeatureParameter {
        feature_id: feature.0,
        parameter_path: path.into(),
        value_type: AssistantCadParameterValueType::Length,
        value: value.into(),
    }
}

/// A coordinate may be zero or negative; whether a length must be positive is
/// decided by the model for each parameter.
#[test]
fn a_coordinate_takes_zero_or_a_negative_value_and_the_model_judges_lengths() {
    let mut session = DocumentSession::default();
    commit(&mut session, vec![super::document_session::part()]);
    let center_x = |snapshot: &Snapshot, sketch: FeatureId| {
        let target =
            FeatureParameterTarget::new(sketch, "entities.1.center.x", ParameterValueType::Length)
                .unwrap();
        snapshot.feature_parameter_value(&target)
    };
    let snapshot = session.snapshot();
    let sketch = snapshot
        .features()
        .map(|feature| feature.id())
        .find(|id| center_x(&snapshot, *id).is_some())
        .expect("the part's circle sketch");
    assert_eq!(center_x(&snapshot, sketch), Some(0.0));

    commit(
        &mut session,
        vec![set_length(sketch, "entities.1.center.x", -10.0)],
    );
    assert_eq!(center_x(&session.snapshot(), sketch), Some(-10.0));
    commit(
        &mut session,
        vec![set_length(sketch, "entities.1.center.x", 0.0)],
    );
    assert_eq!(center_x(&session.snapshot(), sketch), Some(0.0));

    // A negative blind extrusion sweeps against the profile normal.
    let feature = extrusion(&session.snapshot());
    let distance =
        FeatureParameterTarget::new(feature, "extent.distance", ParameterValueType::Length)
            .unwrap();
    commit(
        &mut session,
        vec![set_length(feature, "extent.distance", -5.0)],
    );
    assert_eq!(
        session.snapshot().feature_parameter_value(&distance),
        Some(-5.0)
    );

    // Whether a length must be positive is the model's rule for that parameter.
    let digest = session.snapshot().canonical_digest();
    for refused in [
        set_length(feature, "extent.distance", 0.0),
        set_length(sketch, "entities.1.radius", 0.0),
        set_length(sketch, "entities.1.radius", -3.0),
    ] {
        let error = session
            .plan_cad_program(&program(vec![refused.clone()]), &BTreeSet::new())
            .expect_err(&format!("{refused:?} must be refused"));
        assert!(
            matches!(error, SessionError::Planning(_) | SessionError::Prepare(_)),
            "{refused:?}: {error:?}"
        );
        assert_eq!(session.snapshot().canonical_digest(), digest);
    }
}

#[test]
fn dimensions_and_inputs_that_do_not_resolve_are_refused_without_a_change() {
    let mut session = DocumentSession::default();
    commit(&mut session, vec![super::document_session::part()]);
    let feature = extrusion(&session.snapshot());
    commit(
        &mut session,
        vec![
            height(90, feature, "extent.distance"),
            Operation::CreateEvaluatorInput {
                node_id: 7,
                name: "shelf_depth".to_owned(),
                value: 300.0,
            },
        ],
    );
    let digest = session.snapshot().canonical_digest();
    let undo = session.visible_undo_steps();
    let refused = |operation: Operation| {
        let error = session
            .plan_cad_program(&program(vec![operation.clone()]), &BTreeSet::new())
            .expect_err(&format!("{operation:?} must be refused"));
        assert!(
            matches!(error, SessionError::Planning(_) | SessionError::Prepare(_)),
            "{operation:?}: {error:?}"
        );
        assert_eq!(session.snapshot().canonical_digest(), digest);
        assert_eq!(session.visible_undo_steps(), undo);
    };
    // A feature that does not exist, a parameter it does not have, a bad path.
    refused(height(91, FeatureId(999), "extent.distance"));
    refused(height(91, feature, "extent.width"));
    refused(height(91, feature, "Extent Distance"));
    // Reserved id, an empty name, too many decimal places.
    refused(height(0, feature, "extent.distance"));
    let mut unnamed = height(91, feature, "extent.distance");
    if let Operation::UpsertDimension { name, .. } = &mut unnamed {
        name.clear();
    }
    refused(unnamed);
    let mut precise = height(91, feature, "extent.distance");
    if let Operation::UpsertDimension { decimal_places, .. } = &mut precise {
        *decimal_places = Some(10);
    }
    refused(precise);
    // A dimension that does not exist.
    refused(Operation::DeleteDimension { dimension_id: 99 });
    // An input that does not exist, a non-finite value.
    refused(Operation::SetEvaluatorInput {
        node_id: 8,
        value: 1.0,
    });
    refused(Operation::SetEvaluatorInput {
        node_id: 7,
        value: f64::NAN,
    });
}
