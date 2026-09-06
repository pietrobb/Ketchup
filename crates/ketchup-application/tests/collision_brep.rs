use ketchup_application::validation::{
    CollisionScope, assistant_validation_context, assistant_validation_context_with_worker,
    scoped_collision_report_with_worker,
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

fn exact_scope(document: &DocumentStore, scope: &CollisionScope) -> serde_json::Value {
    scoped_collision_report_with_worker(
        &document.current(),
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
        scope,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        assert_eq!(
            report["collision"]["broad_phase_rejected_pair_count"],
            usize::from(x == 11.0),
            "{report}"
        );
        assert_eq!(
            report["collision"]["narrow_phase_pair_count"],
            usize::from(x != 11.0),
            "{report}"
        );
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
fn scoped_collision_checks_boundary_neighbors_but_rejects_distant_pairs() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    add(&mut document, 2, rectangle(), 9.0);
    add(&mut document, 3, rectangle(), 100.0);
    let scope = CollisionScope::bind(&document.current(), [OccurrenceId(1)]);

    let report = exact_scope(&document, &scope);
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["issue_count"], 1, "{report}");
    assert_eq!(report["total_body_count"], 1, "{report}");
    assert_eq!(report["model_body_count"], 3, "{report}");
    assert_eq!(report["total_pair_count"], 2, "{report}");
    assert_eq!(report["checked_pair_count"], 2, "{report}");
    assert_eq!(report["broad_phase_rejected_pair_count"], 1, "{report}");
    assert_eq!(report["narrow_phase_pair_count"], 1, "{report}");
    assert_eq!(report["scope"]["boundary_occurrence_count"], 1, "{report}");
    assert_eq!(
        report["scope"]["candidate_coverage_complete"], true,
        "{report}"
    );
    assert_eq!(report["issues"][0]["right_occurrence_id"], 2, "{report}");
}

#[test]
fn scoped_collision_rejects_all_distant_pairs_across_ten_thousand_occurrences() {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Repeated part".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(1),
            definition_id: DefinitionId(1),
            name: "Profile".into(),
            kind: FeatureKind::Profile {
                points_mm: rectangle(),
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(2),
            definition_id: DefinitionId(1),
            name: "Solid".into(),
            kind: FeatureKind::Extrusion {
                profile: FeatureId(1),
                height: Dimension::new("10", 10.0).unwrap(),
            },
        },
    ];
    commands.extend((1..=10_000).map(|id| CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(id),
        definition_id: DefinitionId(1),
        name: format!("Part {id}"),
        transform: Transform::from_translation(id as f64 * 20.0, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    }));
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let scope = CollisionScope::bind(&document.current(), [OccurrenceId(5_000)]);

    let report = exact_scope(&document, &scope);
    assert_eq!(report["state"], "passed", "{report}");
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["model_body_count"], 10_000);
    assert_eq!(report["total_body_count"], 1);
    assert_eq!(report["total_pair_count"], 9_999);
    assert_eq!(report["checked_pair_count"], 9_999);
    assert_eq!(report["broad_phase_rejected_pair_count"], 9_999);
    assert_eq!(report["narrow_phase_pair_count"], 0);
    assert_eq!(report["scope"]["boundary_occurrence_count"], 0);
}

#[test]
fn scoped_collision_caps_unique_graph_preparation() {
    let mut document = DocumentStore::new();
    let commands = (1..=513)
        .flat_map(|id| {
            [
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(id),
                    name: format!("Part {id}"),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(id * 2 - 1),
                    definition_id: DefinitionId(id),
                    name: "Profile".into(),
                    kind: FeatureKind::Profile {
                        points_mm: rectangle(),
                    },
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
                    transform: Transform::from_translation(id as f64 * 20.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]
        })
        .collect::<Vec<_>>();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let scope = CollisionScope::bind(&document.current(), [OccurrenceId(1)]);

    let report = exact_scope(&document, &scope);
    assert_eq!(report["state"], "not_evaluated", "{report}");
    assert_eq!(report["complete"], false, "{report}");
    assert_eq!(
        report["not_evaluated"][0]["reason"], "exact_graph_count_resource_limit",
        "{report}"
    );
    assert_eq!(report["not_evaluated"][0]["limit"], 512, "{report}");
}

#[test]
fn scoped_collision_cancel_is_explicit_and_incomplete() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    let scope = CollisionScope::bind(&document.current(), [OccurrenceId(1)]);
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let report = scoped_collision_report_with_worker(
        &document.current(),
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
        &scope,
        cancelled,
    );
    assert_eq!(report["state"], "not_evaluated", "{report}");
    assert_eq!(report["complete"], false, "{report}");
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "exact_collision_cancelled"
    );
}

#[test]
fn full_validation_context_fails_closed_at_its_scene_projection_budget() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    document
        .apply_batch(&CommandBatch::new(
            (2..=101)
                .map(|id| CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(id),
                    definition_id: DefinitionId(1),
                    name: format!("Part {id}"),
                    transform: Transform::from_translation(id as f64 * 20.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tag: None,
                    visible: true,
                })
                .collect(),
        ))
        .unwrap();

    let report = assistant_validation_context(
        &document.current(),
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::only(&["gravity_support"]),
    );
    assert_eq!(report["state"], "not_evaluated", "{report}");
    assert_eq!(report["complete"], false, "{report}");
    assert_eq!(report["visible_occurrence_count"], serde_json::Value::Null);
    assert_eq!(report["visible_occurrence_count_at_least"], 101, "{report}");
    assert_eq!(
        report["validation_context_resource_limit"]["resource"], "Occurrences",
        "{report}"
    );
    assert_eq!(
        report["validation_context_resource_limit"]["limit"], 100,
        "{report}"
    );
    assert!(
        report["not_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["validator"] == "gravity_support"
                    && entry["reason"] == "validation_context_resource_limit"
            }),
        "{report}"
    );
}

#[test]
fn scoped_collision_rejects_a_scope_bound_to_an_older_snapshot() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    let scope = CollisionScope::bind(&document.current(), [OccurrenceId(1)]);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();

    let report = exact_scope(&document, &scope);
    assert_eq!(report["state"], "not_evaluated", "{report}");
    assert_eq!(report["complete"], false, "{report}");
    assert_eq!(report["issue_count"], 0, "{report}");
    assert_eq!(
        report["not_evaluated"][0]["reason"], "stale_collision_scope",
        "{report}"
    );
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
