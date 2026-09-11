use std::collections::{BTreeMap, BTreeSet};

use ketchup_application::{
    AssistantValidationSelection, DocumentSession, StructuralValidationScope,
    scoped_static_load_report,
    validation::{assistant_static_load_report, assistant_validation_context},
};
use ketchup_core::{
    document::*, exact_product::ExactResultRegistry, validation::ValidatorRoleIndex,
};

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
fn global_static_load_reports_every_role_and_load_declaration_as_incomplete() {
    let mut document = structural_document(6);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(3),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(4),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(7),
                name: "physics.applied_load_n.occurrence.3".into(),
                dimension: Dimension::new("200", 200.0).unwrap(),
                dependencies: Vec::new(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(8),
                name: "physics.applied_load_n.occurrence.5".into(),
                dimension: Dimension::new("200", 200.0).unwrap(),
                dependencies: Vec::new(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(9),
                name: "physics.mass_kg.occurrence.6".into(),
                dimension: Dimension::new("100", 100.0).unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let names = snapshot
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let roles = ValidatorRoleIndex::from_snapshot(&snapshot);

    let report = assistant_static_load_report(&snapshot, &names, &roles, true, true);

    assert_eq!(report["state"], "not_evaluated", "{report:#}");
    assert_eq!(report["complete"], false, "{report:#}");
    assert_eq!(report["applicable_count"], 0, "{report:#}");
    assert_eq!(report["not_evaluated"].as_array().unwrap().len(), 5);
    for (index, occurrence_id, reason) in [
        (0, 3, "missing_or_ambiguous_mass"),
        (1, 4, "missing_or_ambiguous_mass"),
        (2, 5, "missing_or_invalid_static_load_role"),
        (3, 6, "missing_or_invalid_static_load_role"),
        (4, 1, "incomplete_static_load_case"),
    ] {
        assert_eq!(
            report["not_evaluated"][index]["occurrence_id"], occurrence_id,
            "{report:#}"
        );
        assert_eq!(
            report["not_evaluated"][index]["reason"], reason,
            "{report:#}"
        );
    }
}

#[test]
fn shared_support_capacity_aggregates_by_load_case_with_scoped_global_parity() {
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
    let names = snapshot
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let roles = ValidatorRoleIndex::from_snapshot(&snapshot);
    let global_report = assistant_static_load_report(&snapshot, &names, &roles, true, true);

    for aggregate in [&report, &global_report] {
        assert_eq!(aggregate["state"], "failed", "{aggregate:#}");
        assert_eq!(aggregate["complete"], true, "{aggregate:#}");
        assert_eq!(aggregate["applicable_count"], 2);
        assert_eq!(aggregate["issue_count"], 1);
        assert_eq!(
            aggregate["evaluations"][0]["case_load_occurrence_ids"],
            serde_json::json!([1, 2])
        );
        assert_eq!(aggregate["evaluations"][0]["resultant_force_n"], 1_181.0);
        assert_eq!(
            aggregate["evaluations"][0]["case_resultant_force_n"],
            2_362.0
        );
        assert_eq!(
            aggregate["evaluations"][0]["total_support_capacity_n"],
            2_000.0
        );
        assert_eq!(aggregate["evaluations"][0]["capacity_margin_n"], -362.0);
        assert_eq!(
            aggregate["issues"][0]["load_occurrence_ids"],
            serde_json::json!([1, 2])
        );
        assert_eq!(aggregate["issues"][0]["capacity_shortfall_n"], 362.0);
    }
    assert_eq!(report["evaluations"], global_report["evaluations"]);
    assert_eq!(report["issues"], global_report["issues"]);

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
fn furniture_validators_use_nonuniform_occurrence_scale_and_reject_shear() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Shelf".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Shelf profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [500.0, 0.0], [500.0, 300.0], [0.0, 300.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Shelf solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("20", 20.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Scaled shelf".into(),
                transform: Transform::from_matrix([
                    2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ])
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Case".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(2),
                name: "Case profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [400.0, 0.0], [400.0, 400.0], [0.0, 400.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(2),
                name: "Case solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(3),
                    height: Dimension::new("800", 800.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(2),
                name: "Scaled case".into(),
                transform: Transform::from_matrix([
                    0.5, 0.0, 0.0, 2_000.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0,
                    1.0,
                ])
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(1),
                name: "ketchup.validator-role.v1".into(),
                categories: vec![
                    (ClassificationCategoryId(1), "furniture.shelf.xy".into()),
                    (ClassificationCategoryId(2), "furniture.case.z".into()),
                ],
            },
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
        ]))
        .unwrap();
    let selection =
        AssistantValidationSelection::only(&["shelf_deflection", "tipping", "anchoring"]);
    let exact_results = ExactResultRegistry::default();
    let snapshot = document.current();
    let report = assistant_validation_context(&snapshot, &exact_results, &selection);

    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["state"], "failed", "{report:#}");
    let shelf = &report["shelf_deflection"];
    assert_eq!(shelf["state"], "failed", "{shelf:#}");
    assert_eq!(shelf["evaluations"][0]["span_mm"], 1_000.0);
    assert_eq!(shelf["evaluations"][0]["depth_mm"], 300.0);
    assert_eq!(shelf["evaluations"][0]["thickness_mm"], 20.0);
    assert_eq!(shelf["evaluations"][0]["result"], "failed");
    let tipping = &report["tipping"];
    assert_eq!(tipping["state"], "failed", "{tipping:#}");
    assert_eq!(tipping["evaluations"][0]["base_depth_mm"], 200.0);
    assert_eq!(tipping["evaluations"][0]["height_mm"], 1_600.0);
    assert_eq!(tipping["evaluations"][0]["result"], "failed");
    let anchoring = &report["anchoring"];
    assert_eq!(anchoring["state"], "failed", "{anchoring:#}");
    assert_eq!(anchoring["evaluations"][0]["height_depth_ratio"], 8.0);
    assert_eq!(anchoring["evaluations"][0]["anchoring_required"], true);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_matrix([
                    1.0, 0.25, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ])
                .unwrap(),
            },
        ]))
        .unwrap();
    let sheared = assistant_validation_context(&document.current(), &exact_results, &selection);
    assert_eq!(sheared["shelf_deflection"]["state"], "not_evaluated");
    assert_eq!(sheared["shelf_deflection"]["complete"], false);
    assert_eq!(
        sheared["shelf_deflection"]["not_evaluated"][0]["reason"],
        "occurrence transform shears the declared source frame"
    );
}

#[test]
fn gravity_support_does_not_propagate_through_an_unproven_envelope_contact() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Boolean support".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Outer profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Outer solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "Opening profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[20.0, 20.0], [80.0, 20.0], [80.0, 80.0], [20.0, 80.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Opening solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(3),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DefinitionId(1),
                name: "Cut support".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    target: FeatureId(2),
                    tool: FeatureId(4),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Ground support".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Load".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(2),
                name: "Load profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(7),
                definition_id: DefinitionId(2),
                name: "Load solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(6),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(2),
                name: "Load over opening".into(),
                transform: Transform::from_translation(40.0, 40.0, 10.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(3),
                definition_id: DefinitionId(2),
                name: "Transitive load".into(),
                transform: Transform::from_translation(40.0, 40.0, 20.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let report = assistant_validation_context(
        &snapshot,
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::only(&["gravity_support"]),
    );

    assert_eq!(report["state"], "not_evaluated", "{report:#}");
    assert_eq!(report["complete"], false, "{report:#}");
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "exact_gravity_contact_unavailable"
    );
    assert_eq!(report["gravity_support"]["state"], "not_evaluated");
    assert_eq!(report["gravity_support"]["complete"], false);
    assert_eq!(report["gravity_support"]["unsupported_count"], 0);
    assert_eq!(report["gravity_support"]["unproven_contact_count"], 2);
    assert_eq!(
        report["gravity_support"]["issues"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        report["gravity_support"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .all(|issue| issue["code"] == "gravity.contact-unproven")
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
