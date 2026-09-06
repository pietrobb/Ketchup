use std::collections::BTreeSet;

use ketchup_application::{
    AssistantValidationSelection, DocumentSession, StructuralValidationScope,
    scoped_static_load_report,
};
use ketchup_core::document::*;

fn structural_document(occurrence_count: u64) -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Structural participant".into(),
        },
        CanonicalCommand::UpsertClassificationDimension {
            id: ClassificationDimensionId(1),
            name: "ketchup.validator-role.v1".into(),
            categories: vec![
                (
                    ClassificationCategoryId(1),
                    "physics.static.load:case-a".into(),
                ),
                (
                    ClassificationCategoryId(2),
                    "physics.static.support:case-a".into(),
                ),
            ],
        },
    ];
    commands.extend(
        (1..=occurrence_count).map(|id| CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(1),
            name: format!("Structural occurrence {id}"),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }),
    );
    commands.extend([
        CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(1),
            dimension_id: ClassificationDimensionId(1),
            category_id: Some(ClassificationCategoryId(1)),
        },
        CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(2),
            dimension_id: ClassificationDimensionId(1),
            category_id: Some(ClassificationCategoryId(2)),
        },
    ]);
    for (id, name, value) in [
        (1, "physics.gravity_x_m_s2", 0.0),
        (2, "physics.gravity_y_m_s2", 0.0),
        (3, "physics.gravity_z_m_s2", -9.81),
        (4, "physics.mass_kg.occurrence.1", 100.0),
        (5, "physics.applied_load_n.occurrence.1", 200.0),
        (6, "physics.support_capacity_n.occurrence.2", 2_000.0),
    ] {
        commands.push(CanonicalCommand::CreateEvaluatorNode {
            id: NodeId(id),
            name: name.into(),
            dimension: Dimension::new(value.to_string(), value).unwrap(),
            dependencies: Vec::new(),
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

#[test]
fn unchecked_public_selection_cannot_report_success_without_a_known_validator() {
    let session = DocumentSession::default();
    for requested in [
        BTreeSet::from(["bogus"]),
        BTreeSet::from(["bogus", "collision"]),
    ] {
        let selection = AssistantValidationSelection {
            mode: "only",
            requested,
            unknown: Vec::new(),
        };
        assert!(!selection.is_valid());
        let report = session.validators(&selection);
        assert_eq!(report["state"], "not_evaluated");
        assert_eq!(report["complete"], false);
        assert_eq!(report["executed"], serde_json::json!([]));
        assert_eq!(
            report["selection_error"],
            "unknown_or_empty_validator_selection"
        );
    }
    assert_eq!(session.visible_undo_steps(), 0);
}

#[test]
fn structural_scope_is_deterministic_and_stale_after_mutation() {
    let mut document = DocumentStore::new();
    let scope = StructuralValidationScope::bind(
        &document.current(),
        [OccurrenceId(2), OccurrenceId(1), OccurrenceId(2)],
    );
    assert!(scope.is_current(&document.current()));
    assert_eq!(
        scope.occurrence_ids(),
        &BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: TagId(1),
            name: "revision change".into(),
            visible: true,
        }]))
        .unwrap();
    assert!(!scope.is_current(&document.current()));
    let report = scoped_static_load_report(&document.current(), &scope, || false);
    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "stale_structural_scope"
    );
}

#[test]
fn structural_scope_binding_stops_at_resource_limit() {
    let document = DocumentStore::new();
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, (1..=10_001).map(OccurrenceId));

    assert_eq!(scope.occurrence_ids().len(), 10_000);
    let report = scoped_static_load_report(&snapshot, &scope, || false);
    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "structural_scope_resource_limit"
    );
}

#[test]
fn scoped_static_load_includes_boundary_support_without_exact_geometry() {
    let document = structural_document(2);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["applicable_count"], 1);
    assert_eq!(report["evaluations"][0]["occurrence_id"], 1);
    assert_eq!(report["evaluations"][0]["supports"][0]["occurrence_id"], 2);
    assert_eq!(
        report["coverage"]["boundary_support_occurrence_ids"],
        serde_json::json!([2])
    );
    assert_eq!(report["coverage"]["exact_geometry_loaded_count"], 0);
}

#[test]
fn scoped_static_load_cancellation_is_incomplete() {
    let document = structural_document(2);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let mut polls = 0;
    let report = scoped_static_load_report(&snapshot, &scope, || {
        polls += 1;
        polls > 11
    });

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(report["not_evaluated"][0]["reason"], "validation_cancelled");
    assert_eq!(report["coverage"]["checked_load_occurrence_count"], 0);
}

#[test]
fn scoped_static_load_missing_role_is_incomplete() {
    let document = structural_document(3);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(3)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "missing_or_invalid_static_load_role"
    );
}

#[test]
fn scoped_static_load_missing_input_is_incomplete() {
    let mut document = structural_document(2);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameEvaluatorNode {
                id: NodeId(4),
                name: "unrelated parameter".into(),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated", "{report:#}");
    assert_eq!(report["complete"], false, "{report:#}");
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "missing_or_ambiguous_mass"
    );
}

#[test]
fn scoped_static_load_rejects_shared_support_capacity_without_aggregation() {
    let mut document = structural_document(3);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(2),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(3),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(2)),
            },
            CanonicalCommand::RenameEvaluatorNode {
                id: NodeId(6),
                name: "physics.support_capacity_n.occurrence.3".into(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(7),
                name: "physics.mass_kg.occurrence.2".into(),
                dimension: Dimension::new("100", 100.0).unwrap(),
                dependencies: Vec::new(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(8),
                name: "physics.applied_load_n.occurrence.2".into(),
                dimension: Dimension::new("200", 200.0).unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1), OccurrenceId(2)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "shared_static_load_case_requires_aggregate_evaluation"
    );

    let isolated_scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);
    let isolated_report = scoped_static_load_report(&snapshot, &isolated_scope, || false);
    assert_eq!(isolated_report["state"], "not_evaluated");
    assert_eq!(isolated_report["complete"], false);
    assert_eq!(
        isolated_report["not_evaluated"][0]["reason"],
        "shared_static_load_case_outside_scope"
    );
}

#[test]
fn scoped_static_load_rejects_more_loads_than_can_be_reported() {
    let document = structural_document(101);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, (1..=101).map(OccurrenceId));

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "structural_load_resource_limit"
    );
}

#[test]
fn scoped_static_load_rejects_over_budget_output_text() {
    let mut document = structural_document(2);
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameEntity {
            id: OccurrenceId(1),
            name: "x".repeat(4 * 1024 * 1024 + 1),
        }]))
        .unwrap();
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "structural_text_resource_limit"
    );
}

#[test]
fn scoped_static_load_handles_ten_thousand_sparse_occurrences() {
    let document = structural_document(10_000);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["coverage"]["requested_occurrence_count"], 1);
    assert_eq!(report["coverage"]["checked_load_occurrence_count"], 1);
    assert_eq!(report["coverage"]["exact_geometry_loaded_count"], 0);
}

#[test]
fn canonical_catalog_selections_remain_valid() {
    assert!(AssistantValidationSelection::all("all").is_valid());
    assert!(AssistantValidationSelection::only(&["gravity_support"]).is_valid());
    assert!(!AssistantValidationSelection::only(&[]).is_valid());
    assert!(!AssistantValidationSelection::only(&["bogus"]).is_valid());
}
