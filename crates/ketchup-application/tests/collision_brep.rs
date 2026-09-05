use ketchup_application::validation::{
    assistant_validation_context, assistant_validation_context_with_worker,
};
use ketchup_application::{AssistantValidationSelection, DocumentSession, SessionSettings};
use ketchup_core::{document::*, exact_product::ExactResultRegistry, persistence::ContainerData};
use std::time::Duration;

fn add(document: &mut DocumentStore, id: u64, points: Vec<[f64; 2]>, x: f64) {
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(id),
                name: format!("Part {id}"),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(id * 2 - 1),
                definition_id: DefinitionId(id),
                name: "Profile".into(),
                kind: FeatureKind::Profile { points_mm: points },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(id * 2),
                definition_id: DefinitionId(id),
                name: "Solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(id * 2 - 1),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(id),
                name: format!("Part {id}"),
                transform: Transform::from_translation(x, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
}
fn rectangle() -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]
}
fn selection() -> AssistantValidationSelection {
    AssistantValidationSelection::only(&["collision"])
}
fn exact(document: &DocumentStore) -> serde_json::Value {
    assistant_validation_context_with_worker(
        &document.current(),
        &ExactResultRegistry::default(),
        &selection(),
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
    )
}
#[test]
fn worker_contact_penetration_and_snapshot_are_exact() {
    for (x, state, count) in [(10.0, "passed", 0), (9.0, "failed", 1), (11.0, "passed", 0)] {
        let mut document = DocumentStore::new();
        add(&mut document, 1, rectangle(), 0.0);
        add(&mut document, 2, rectangle(), x);
        let before = document.current();
        let undo = document.visible_undo_steps();
        let report = exact(&document);
        assert_eq!(report["state"], state, "{report}");
        assert_eq!(report["complete"], true, "{report}");
        assert_eq!(report["issue_count"], count);
        assert_eq!(report["checked_pair_count"], 1);
        if count == 1 {
            assert_eq!(report["issues"][0]["left"]["producer_feature_id"], 2);
            assert!(report["issues"][0]["left_instance_path"]["steps"].is_array());
        }
        assert_eq!(
            before.canonical_digest(),
            document.current().canonical_digest()
        );
        assert_eq!(undo, document.visible_undo_steps());
    }
}
#[test]
fn overlapping_sloped_envelopes_are_not_solid_collisions() {
    let mut document = DocumentStore::new();
    add(
        &mut document,
        1,
        vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
        0.0,
    );
    add(
        &mut document,
        2,
        vec![[10.0, 10.0], [1.0, 10.0], [10.0, 1.0]],
        0.0,
    );
    let exact = exact(&document);
    assert_eq!(exact["state"], "passed", "{exact}");
    assert_eq!(exact["complete"], true);
    let legacy = assistant_validation_context(
        &document.current(),
        &ExactResultRegistry::default(),
        &selection(),
    );
    assert_eq!(legacy["state"], "not_evaluated");
    assert_eq!(legacy["issue_count"], 0);
    assert_eq!(legacy["complete"], false);
}
#[test]
fn missing_worker_and_partial_analytic_coverage_never_pass() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    add(&mut document, 2, rectangle(), 1.0);
    let missing = assistant_validation_context_with_worker(
        &document.current(),
        &ExactResultRegistry::default(),
        &selection(),
        &ContainerData::default(),
        Some("C:/no-such-worker.exe".into()),
        Duration::from_secs(2),
    );
    assert_eq!(missing["state"], "not_evaluated");
    assert_eq!(missing["complete"], false);
    assert_eq!(missing["issue_count"], 0);
    add(
        &mut document,
        3,
        vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
        100.0,
    );
    let legacy = assistant_validation_context(
        &document.current(),
        &ExactResultRegistry::default(),
        &selection(),
    );
    assert_eq!(legacy["state"], "failed", "{legacy}");
    assert_eq!(legacy["complete"], false);
    assert_eq!(legacy["issue_count"], 1);
}
#[test]
fn full_140_house_has_no_silent_collision_cap() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/garden-studio-colored.ketchup");
    let session = DocumentSession::open(
        path,
        SessionSettings {
            evaluation_timeout: Duration::from_secs(600),
            ..SessionSettings::default()
        },
    )
    .unwrap();
    let before = session.snapshot();
    let report = session.validators(&selection());
    assert_eq!(report["visible_occurrence_count"], 140, "{report}");
    assert_eq!(report["checked_occurrence_count"], 140, "{report}");
    assert_eq!(report["checked_pair_count"], 9730, "{report}");
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(
        session.snapshot().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(session.visible_undo_steps(), 0);
    println!("140-house collision issues: {}", report["issue_count"]);
}
#[test]
fn exact_hole_does_not_collide_with_insert() {
    use ketchup_core::assistant_sidecar::*;
    let mut session = DocumentSession::default();
    let mut operations = Vec::new();
    for (name, radii) in [("ring", vec![10.0, 5.0]), ("insert", vec![4.0])] {
        operations.push(AssistantCadEditOperation::CreatePart {
            name: name.into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: radii
                .iter()
                .enumerate()
                .map(|(i, r)| AssistantSketchEntity::Circle {
                    id: i as u64 + 1,
                    center_mm: [0.0, 0.0],
                    radius_mm: *r,
                })
                .collect(),
            constraints: vec![],
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 10.0 },
            translation_mm: [0.0, 0.0, 0.0],
            rotation: None,
        });
    }
    session
        .apply_cad_program(
            &AssistantCadEditProgram { operations },
            &std::collections::BTreeSet::new(),
        )
        .unwrap();
    let report = session.validators(&selection());
    assert_eq!(report["state"], "passed", "{report}");
    assert_eq!(report["complete"], true);
}

#[test]
fn regression_hidden_overlapping_body_is_not_reported() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    add(&mut document, 2, rectangle(), 1.0);
    add(&mut document, 3, rectangle(), 30.0);
    let before = assistant_validation_context(
        &document.current(),
        &ExactResultRegistry::default(),
        &selection(),
    );
    assert_eq!(before["collision"]["issue_count"], 1, "{before}");
    let body_id = *ketchup_core::exact_product::exact_body_terminal_features(
        &document.current(),
        DefinitionId(2),
    )
    .unwrap()
    .keys()
    .next()
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBodyVisibility {
                definition_id: DefinitionId(2),
                id: body_id,
                visible: false,
            },
        ]))
        .unwrap();
    let report = assistant_validation_context(
        &document.current(),
        &ExactResultRegistry::default(),
        &selection(),
    );
    assert_eq!(report["visible_occurrence_count"], 3, "{report}");
    assert_eq!(report["state"], "passed", "{report}");
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["collision"]["total_body_count"], 2);
    assert_eq!(report["collision"]["checked_body_count"], 2);
    assert_eq!(report["checked_pair_count"], 1);
    assert_eq!(report["issue_count"], 0);
    assert_eq!(report["collision"]["issues"], serde_json::json!([]));
    assert_eq!(
        report["collision"]["unavailable_occurrences"],
        serde_json::json!([])
    );
    assert_eq!(report["not_evaluated"], serde_json::json!([]));
}

#[test]
fn regression_empty_container_checks_projected_child_solid_completely() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    add(&mut document, 2, rectangle(), 1.0);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(10),
                name: "Assembly".into(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(GroupId(10)),
            },
        ]))
        .unwrap();
    let component = document
        .convert_group_to_component(GroupId(10), "Container")
        .unwrap();
    let snapshot = document.current();
    let container = snapshot
        .scene_query()
        .into_iter()
        .find(|occurrence| {
            occurrence.instance_path.root_occurrence() == component.component_occurrence_id
                && occurrence.instance_path.steps().is_empty()
        })
        .unwrap();
    assert!(
        ketchup_core::exact_product::exact_body_terminal_features(
            &snapshot,
            container.definition_id,
        )
        .unwrap()
        .is_empty()
    );
    let report = exact(&document);
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["checked_occurrence_count"], 2);
    assert_eq!(report["collision"]["checked_body_count"], 2);
    assert_eq!(report["checked_pair_count"], 1);
    assert_eq!(report["issue_count"], 1);
    assert_eq!(
        report["collision"]["unavailable_occurrences"],
        serde_json::json!([])
    );
    assert_eq!(report["collision"]["not_evaluated"], serde_json::json!([]));
    assert_eq!(report["not_evaluated"], serde_json::json!([]));
    let issue = &report["issues"][0];
    assert!(
        ["left", "right"].into_iter().any(|side| {
            issue[side]["instance_path"]["root_occurrence_id"]
                == component.component_occurrence_id.0
                && !issue[side]["instance_path"]["steps"]
                    .as_array()
                    .unwrap()
                    .is_empty()
        }),
        "{report}"
    );
}

#[test]
fn regression_partial_analytic_collision_retains_failure_and_incomplete_evidence() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    add(&mut document, 2, rectangle(), 1.0);
    add(
        &mut document,
        3,
        vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
        100.0,
    );
    let report = assistant_validation_context(
        &document.current(),
        &ExactResultRegistry::default(),
        &selection(),
    );
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["complete"], false, "{report}");
    let collision = &report["collision"];
    assert_eq!(collision["state"], "failed", "{report}");
    assert_eq!(collision["complete"], false);
    assert_eq!(collision["checked_pair_count"], 1);
    assert_eq!(collision["total_pair_count"], 3);
    assert_eq!(collision["issue_count"], 1);
    assert_eq!(collision["issues"][0]["code"], "collision.detected");
    assert_eq!(
        collision["issues"][0]["evidence"]["method"],
        "canonical_box_analytic"
    );
    assert_eq!(report["issues"], collision["issues"]);
    assert_eq!(collision["unavailable_occurrences"][0]["occurrence_id"], 3);
    assert_eq!(
        collision["unavailable_occurrences"][0]["reason"],
        "exact_worker_required_for_non_box_geometry"
    );
    assert!(
        collision["not_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["reason"] == "incomplete_exact_geometry_coverage" }),
        "{report}"
    );
    assert!(
        collision["not_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["reason"] == "incomplete_exact_pair_coverage"
                    && entry["unchecked_pair_count"] == 2
            }),
        "{report}"
    );
    assert!(
        report["not_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["validator"] == "collision" }),
        "{report}"
    );
}

#[test]
fn regression_all_validators_keep_known_collision_failed_when_others_unavailable() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    add(&mut document, 2, rectangle(), 1.0);
    let report = assistant_validation_context(
        &document.current(),
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::all("all"),
    );
    assert_eq!(report["collision"]["state"], "failed", "{report}");
    assert_eq!(report["collision"]["complete"], true);
    assert_eq!(report["collision"]["issue_count"], 1);
    assert_eq!(
        report["gravity_support"]["state"], "not_evaluated",
        "{report}"
    );
    assert!(
        report["not_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["validator"] == "gravity_support" }),
        "{report}"
    );
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["complete"], false);
    assert_eq!(report["issues"], report["collision"]["issues"]);
    assert_eq!(report["issues"][0]["code"], "collision.detected");
}
