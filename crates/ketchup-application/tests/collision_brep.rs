use ketchup_application::validation::{
    CollisionScope, FabricationCollisionError, assistant_validation_context,
    assistant_validation_context_with_worker,
    assistant_validation_context_with_worker_cancellation,
    fabrication_collision_validation_with_worker, scoped_collision_report_with_worker,
};
use ketchup_application::{AssistantValidationSelection, DocumentSession, SessionSettings};
use ketchup_model::{
    document::*,
    exact_product::ExactResultRegistry,
    persistence::{self, ContainerData},
    tolerance::TolerancePolicy,
    validation::ValidationState,
};
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
                kind: FeatureKind::polygon(&points),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(id * 2),
                definition_id: DefinitionId(id),
                name: "Solid".into(),
                kind: FeatureKind::extrusion(
                    FeatureId(id * 2 - 1),
                    Dimension::new("10", 10.0).unwrap(),
                ),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(id),
                name: format!("Part {id}"),
                transform: Transform::from_translation(x, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
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
fn fabrication_collision_is_complete_order_independent_and_native() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    add(&mut document, 2, rectangle(), 100.0);
    add(&mut document, 3, rectangle(), 1.0);
    let snapshot = document.current();
    let registry = ExactResultRegistry::default();
    let participants = [3, 1, 2]
        .into_iter()
        .map(|id| {
            ketchup_model::exact_validation::GeneralBodyParticipant::accept(
                &snapshot,
                &registry,
                InstancePath::root(OccurrenceId(id)),
                TolerancePolicy::default(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();

    let validation = fabrication_collision_validation_with_worker(
        &snapshot,
        &participants,
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
    )
    .unwrap();
    // The cases chain the participants (1-2, 2-3); the penetrating pair 1-3 is
    // not among them and is still found, because the native check covers all pairs.
    assert_eq!(validation.cases.len(), 2);
    assert_eq!(validation.report.state, ValidationState::Failed);
    assert_eq!(validation.report.diagnostics.len(), 1);
    assert_eq!(validation.report.diagnostics[0].code, "collision.detected");
    assert!(
        validation.report.diagnostics[0]
            .evidence
            .contains("occt_brep_common_volume")
    );

    let reversed = fabrication_collision_validation_with_worker(
        &snapshot,
        &participants.into_iter().rev().collect::<Vec<_>>(),
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
    )
    .unwrap();
    assert_eq!(
        validation.report.invocation.input_digest,
        reversed.report.invocation.input_digest
    );
    assert_eq!(validation.report.state, reversed.report.state);
}

#[test]
fn a_repeated_fabrication_participant_is_refused_with_its_instance() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    let snapshot = document.current();
    let participant = ketchup_model::exact_validation::GeneralBodyParticipant::accept(
        &snapshot,
        &ExactResultRegistry::default(),
        InstancePath::root(OccurrenceId(1)),
        TolerancePolicy::default(),
    )
    .unwrap();

    let error = fabrication_collision_validation_with_worker(
        &snapshot,
        &[participant.clone(), participant],
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
    )
    .err()
    .expect("a repeated participant must be refused");
    assert_eq!(
        error,
        FabricationCollisionError::DuplicateParticipant(InstancePath::root(OccurrenceId(1)))
    );
}

#[test]
fn scoped_collision_checks_boundary_neighbors_but_rejects_distant_pairs() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    add(&mut document, 2, rectangle(), 9.0);
    add(&mut document, 3, rectangle(), 100.0);
    let scope = CollisionScope::bind(&document.current(), [OccurrenceId(1)]);

    let full = exact(&document);
    let report = exact_scope(&document, &scope);
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["issue_count"], 1, "{report}");
    assert_eq!(
        report["state"], full["state"],
        "scoped={report} full={full}"
    );
    assert_eq!(
        report["complete"], full["complete"],
        "scoped={report} full={full}"
    );
    assert_eq!(
        report["issues"], full["issues"],
        "scoped={report} full={full}"
    );
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
            kind: FeatureKind::polygon(&rectangle()),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(2),
            definition_id: DefinitionId(1),
            name: "Solid".into(),
            kind: FeatureKind::extrusion(FeatureId(1), Dimension::new("10", 10.0).unwrap()),
        },
    ];
    commands.extend((1..=10_000).map(|id| CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(id),
        definition_id: DefinitionId(1),
        name: format!("Part {id}"),
        transform: Transform::from_translation(id as f64 * 20.0, 0.0, 0.0).unwrap(),
        parent: None,
        tags: Default::default(),
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
fn scoped_collision_prepares_more_solids_than_one_worker_batch() {
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
                    kind: FeatureKind::polygon(&rectangle()),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(id * 2),
                    definition_id: DefinitionId(id),
                    name: "Solid".into(),
                    kind: FeatureKind::extrusion(
                        FeatureId(id * 2 - 1),
                        Dimension::new("10", 10.0).unwrap(),
                    ),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(id),
                    definition_id: DefinitionId(id),
                    name: format!("Part {id}"),
                    transform: Transform::from_translation(id as f64 * 20.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                },
            ]
        })
        .collect::<Vec<_>>();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let scope = CollisionScope::bind(&document.current(), [OccurrenceId(1)]);

    // Batches bound the graphs one worker request carries; the whole check may
    // prepare more distinct solids than one batch holds.
    let report = exact_scope(&document, &scope);
    assert_eq!(report["state"], "passed", "{report}");
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["checked_pair_count"], 512, "{report}");
    assert_eq!(report["resource_limits"]["max_graphs_per_batch"], 512);
    assert_eq!(report["resource_limits"]["max_unique_graphs"], 10_000);
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
fn full_assistant_validation_cancellation_is_explicit_and_incomplete() {
    let mut document = DocumentStore::new();
    add(&mut document, 1, rectangle(), 0.0);
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let report = assistant_validation_context_with_worker_cancellation(
        &document.current(),
        &ExactResultRegistry::default(),
        &selection(),
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
        cancelled,
    );
    assert_eq!(report["state"], "not_evaluated", "{report}");
    assert_eq!(report["complete"], false, "{report}");
    assert_eq!(
        report["collision"]["not_evaluated"][0]["reason"],
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
                    tags: Default::default(),
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
fn parallel_worker_failure_cancels_remaining_work_without_masking_the_cause() {
    let _turn = crate::integration_support::file_turn();
    let mut document = DocumentStore::new();
    for id in 1..=20 {
        add(&mut document, id, rectangle(), id as f64 * 100.0);
    }
    let before = document.current();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let report = assistant_validation_context_with_worker_cancellation(
        &before,
        &ExactResultRegistry::default(),
        &selection(),
        &ContainerData::default(),
        Some("C:/no-such-worker.exe".into()),
        Duration::from_secs(10),
        cancelled.clone(),
    );
    assert_eq!(report["complete"], false, "{report}");
    assert!(
        !cancelled.load(std::sync::atomic::Ordering::Acquire),
        "a failed geometry check must not cancel the enclosing request"
    );
    assert_eq!(
        report["collision"]["not_evaluated"][0]["reason"], "exact_worker_unavailable",
        "{report}"
    );
    assert!(
        report["collision"]["not_evaluated"][0]["cause"]
            .as_str()
            .is_some_and(|cause| !cause.is_empty()),
        "{report}"
    );
    assert_eq!(document.current().scene_query(), before.scene_query());
}

#[test]
fn full_140_house_has_no_silent_collision_cap() {
    let _turn = crate::integration_support::file_turn();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/garden-studio-colored.ketchup");
    let legacy = persistence::load_file(&path).unwrap();
    assert_eq!(legacy.source_schema(), 53);
    let legacy_digest = legacy.snapshot().canonical_digest();
    let current = persistence::load(&persistence::save(&legacy.snapshot())).unwrap();
    assert_eq!(current.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(current.snapshot().canonical_digest(), legacy_digest);

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
    use ketchup_assistant::sidecar::*;
    let mut session = DocumentSession::default();
    let mut operations = Vec::new();
    for (name, radii) in [("ring", vec![10.0, 5.0]), ("insert", vec![4.0])] {
        operations.push(AssistantCadEditOperation::CreatePart {
            holes: Vec::new(),
            pockets: Vec::new(),
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

    let snapshot = session.snapshot();
    let fabrication = fabrication_collision_validation_with_worker(
        &snapshot,
        &[],
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
    )
    .unwrap();
    assert_eq!(fabrication.report.state, ValidationState::Passed);
    assert_eq!(fabrication.report.evidence_counts.exact, 1);
    assert!(fabrication.report.diagnostics.is_empty());
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
    let body_id = *ketchup_model::exact_product::exact_body_terminal_features(
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
        ketchup_model::exact_product::exact_body_terminal_features(
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
    assert_eq!(report["gravity_support"]["state"], "passed", "{report}");
    assert!(
        report["not_evaluated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["validator"] == "assembly_retention" }),
        "{report}"
    );
    assert_eq!(report["state"], "failed", "{report}");
    assert_eq!(report["complete"], false);
    assert_eq!(report["issues"], report["collision"]["issues"]);
    assert_eq!(report["issues"][0]["code"], "collision.detected");
}
