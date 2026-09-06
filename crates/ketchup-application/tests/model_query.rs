use ketchup_application::model_query::*;
use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind, PlanarFaceAttachment,
};
use ketchup_core::document::*;
use ketchup_core::exact_product::{
    BODY_SUBSHAPE_REF_SCHEMA_V1, BodySubshapeRef, ExactFaceRole, ReferenceStability,
    canonical_reference_lineage_digest,
};
use serde_json::{Value, json};

fn fixture(count: u64, long_names: bool) -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Repeated component".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(1),
            definition_id: DefinitionId(1),
            name: "Triangle profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
            },
        },
    ];
    commands.extend((1..=count).map(|id| CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(id),
        definition_id: DefinitionId(1),
        name: if long_names {
            format!("{}-{id}", "\\\"".repeat(1000))
        } else {
            format!("part-{id:05}")
        },
        transform: Transform::identity(),
        parent: None,
        tag: None,
        visible: true,
    }));
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}
fn nested_fixture() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Leaf".into(),
            },
            CanonicalCommand::CreateTag {
                id: TagId(7),
                name: "Structural leaf".into(),
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(9),
                name: "Assembly role".into(),
                categories: vec![
                    (ClassificationCategoryId(10), "Primary".into()),
                    (ClassificationCategoryId(11), "Secondary".into()),
                ],
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "Source assembly".into(),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Nested leaf".into(),
                transform: Transform::from_translation(5.0, 0.0, 0.0).unwrap(),
                parent: Some(GroupId(1)),
                tag: Some(TagId(7)),
                visible: true,
            },
        ]))
        .unwrap();
    let converted = document
        .convert_group_to_component(GroupId(1), "Repeated assembly")
        .unwrap();
    let copy_id = OccurrenceId(converted.component_occurrence_id.0 + 1);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: copy_id,
                definition_id: converted.component_definition_id,
                name: "Repeated assembly copy".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: converted.component_occurrence_id,
                dimension_id: ClassificationDimensionId(9),
                category_id: Some(ClassificationCategoryId(10)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: copy_id,
                dimension_id: ClassificationDimensionId(9),
                category_id: Some(ClassificationCategoryId(11)),
            },
        ]))
        .unwrap();
    document
}

fn spatial_fixture() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Bounded part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Unevaluated part".into(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Near bounded".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Far bounded".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(3),
                definition_id: DefinitionId(2),
                name: "Unknown bounds".into(),
                transform: Transform::from_translation(5.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document
}

fn planar_reference(snapshot: &Snapshot) -> BodySubshapeRef {
    let role = ExactFaceRole::Top;
    BodySubshapeRef {
        schema: BODY_SUBSHAPE_REF_SCHEMA_V1.into(),
        document_id: snapshot.document_id(),
        definition_id: DefinitionId(1),
        profile_feature_id: FeatureId(1),
        producer_feature_id: FeatureId(1),
        semantic_role: role.semantic_role().into(),
        source_element_id: role.source_element_id().into(),
        expected_type: role.expected_type().into(),
        expected_cardinality: 1,
        stability: ReferenceStability::Guaranteed,
        canonical_input_digest: "canonical".into(),
        exact_input_digest: "exact".into(),
        result_fingerprint: "result".into(),
        evaluator: "test".into(),
        backend: "test".into(),
        tolerance: "0".into(),
        lineage_digest: canonical_reference_lineage_digest(
            snapshot.document_id(),
            FeatureId(1),
            role.semantic_role(),
            role.source_element_id(),
            role.expected_type(),
        ),
        corroborating_geometry_fingerprint: "geometry".into(),
    }
}

fn request(kind: EntityKind) -> PageRequest {
    PageRequest {
        kind,
        limit: 100,
        search: String::new(),
        definition_id: None,
        tag_id: None,
        classification_dimension_id: None,
        classification_category_id: None,
        world_bounds_mm: None,
        cursor: None,
    }
}
fn bounded(value: &Value) {
    assert!(serde_json::to_vec(value).unwrap().len() < MAX_OUTPUT_BYTES);
}
fn collect(query: &ModelQuery, snapshot: &Snapshot, mut request: PageRequest) -> Vec<u64> {
    let mut ids = Vec::new();
    loop {
        let page = query.page(snapshot, &request).unwrap();
        bounded(&page);
        let items = page["items"].as_array().unwrap();
        assert!(items.len() <= request.limit);
        ids.extend(items.iter().map(|v| v["id"].as_u64().unwrap()));
        if page["complete"] == true {
            assert!(page["next_cursor"].is_null());
            break;
        }
        assert!(!items.is_empty());
        request.cursor = Some(page["next_cursor"].as_str().unwrap().into());
    }
    ids
}

fn collect_instance_ids(
    query: &ModelQuery,
    snapshot: &Snapshot,
    mut request: PageRequest,
) -> Vec<String> {
    let mut ids = Vec::new();
    loop {
        let page = query.page(snapshot, &request).unwrap();
        bounded(&page);
        let items = page["items"].as_array().unwrap();
        assert!(items.len() <= request.limit);
        ids.extend(
            items
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned()),
        );
        if page["complete"] == true {
            assert!(page["next_cursor"].is_null());
            break;
        }
        assert!(!items.is_empty());
        request.cursor = Some(page["next_cursor"].as_str().unwrap().into());
    }
    ids
}

fn collect_relation_ids(
    query: &ModelQuery,
    snapshot: &Snapshot,
    mut request: PageRequest,
) -> (Vec<String>, bool) {
    let mut ids = Vec::new();
    let mut byte_limited = false;
    loop {
        let page = query.page(snapshot, &request).unwrap();
        bounded(&page);
        byte_limited |= page["byte_limited"] == true;
        let items = page["items"].as_array().unwrap();
        assert!(!items.is_empty() || page["complete"] == true);
        ids.extend(
            items
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned()),
        );
        if page["complete"] == true {
            break;
        }
        request.cursor = Some(page["next_cursor"].as_str().unwrap().to_owned());
    }
    (ids, byte_limited)
}

#[test]
fn ten_thousand_repeated_occurrences_are_bounded_without_gaps_or_duplicates() {
    let document = fixture(10_000, false);
    let snapshot = document.current();
    let query = ModelQuery::default();
    let before = snapshot.canonical_digest();
    let undo = document.visible_undo_steps();
    let summary = query.summary(&snapshot);
    bounded(&summary);
    assert_eq!(
        summary["counts"],
        json!({"root_occurrences":10000,"instances":10000,"definitions":1,"features":1,
            "relations":10000})
    );
    assert_eq!(summary["coverage"]["nested_hierarchy"], false);
    assert_eq!(
        collect(&query, &snapshot, request(EntityKind::Occurrences)),
        (1..=10_000).collect::<Vec<_>>()
    );
    let instance_ids = collect_instance_ids(&query, &snapshot, request(EntityKind::Instances));
    assert_eq!(instance_ids.len(), 10_000);
    let mut unique_instance_ids = instance_ids.clone();
    unique_instance_ids.sort_unstable();
    unique_instance_ids.dedup();
    assert_eq!(unique_instance_ids.len(), 10_000);
    let (relation_ids, byte_limited) =
        collect_relation_ids(&query, &snapshot, request(EntityKind::Relations));
    assert!(byte_limited);
    assert_eq!(relation_ids.len(), 10_000);
    let mut unique_relation_ids = relation_ids;
    unique_relation_ids.sort_unstable();
    unique_relation_ids.dedup();
    assert_eq!(unique_relation_ids.len(), 10_000);
    assert_eq!(query.summary(&snapshot), summary);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo);
    assert_eq!(document.visible_redo_steps(), 0);
}

#[test]
fn revision_bound_worksets_hold_ten_thousand_query_identities_and_fail_stale() {
    let mut document = fixture(10_000, false);
    let snapshot = document.current();
    let mut query = ModelQuery::default();
    let created = query
        .create_workset(&snapshot, &request(EntityKind::Occurrences))
        .unwrap();
    bounded(&created);
    assert_eq!(created["scope"]["source"], "model_query");
    assert_eq!(created["scope"]["entity_kind"], "occurrences");
    assert_eq!(created["item_count"], 10_000);
    assert_eq!(created["source_total_matches"], 10_000);
    assert_eq!(created["identity_status"]["missing_identity_count"], 0);
    assert_eq!(created["identity_status"]["stale"], false);
    assert_eq!(created["completeness"]["complete"], true);
    assert_eq!(created["completeness"]["usable_for_batch"], true);
    assert_eq!(created["resource_budget"]["identity_text_bytes"], 38_894);
    let handle = created["workset_handle"].as_str().unwrap().to_owned();
    assert_eq!(query.workset_status(&snapshot, &handle).unwrap(), created);

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameEntity {
            id: OccurrenceId(1),
            name: "changed".into(),
        }]))
        .unwrap();
    query.invalidate();
    assert_eq!(
        query.workset_status(&document.current(), &handle),
        Err(QueryError::StaleWorkset)
    );
}

#[test]
fn worksets_report_incomplete_sources_and_distinguish_missing_handles() {
    let snapshot = spatial_fixture().current();
    let query = ModelQuery::default();
    let mut spatial = request(EntityKind::Instances);
    spatial.world_bounds_mm = Some([[-1.0; 3], [20.0; 3]]);
    let incomplete = query.create_workset(&snapshot, &spatial).unwrap();
    bounded(&incomplete);
    assert_eq!(incomplete["item_count"], 1);
    assert_eq!(incomplete["source_total_matches"], 1);
    assert_eq!(incomplete["completeness"]["complete"], false);
    assert_eq!(incomplete["completeness"]["usable_for_batch"], false);
    assert_eq!(
        incomplete["completeness"]["reason"],
        "source_query_incomplete"
    );

    let mut cursored = request(EntityKind::Occurrences);
    cursored.cursor = Some("not a source scope".into());
    assert_eq!(
        query.create_workset(&snapshot, &cursored),
        Err(QueryError::InvalidInput)
    );
    for limit in [0, MAX_PAGE + 1] {
        let mut invalid = request(EntityKind::Occurrences);
        invalid.limit = limit;
        assert_eq!(
            query.create_workset(&snapshot, &invalid),
            Err(QueryError::InvalidInput)
        );
    }
    assert_eq!(
        query.workset_status(&snapshot, "forged"),
        Err(QueryError::WorksetNotFound)
    );

    let mut handles = Vec::new();
    for _ in 0..=MAX_ACTIVE_WORKSETS {
        handles.push(
            query
                .create_workset(&snapshot, &request(EntityKind::Occurrences))
                .unwrap()["workset_handle"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(
        query.workset_status(&snapshot, &handles[0]),
        Err(QueryError::WorksetNotFound)
    );
    assert!(
        query
            .workset_status(&snapshot, handles.last().unwrap())
            .is_ok()
    );
}

#[test]
fn instance_index_stops_before_over_budget_projection_and_reports_incomplete() {
    let document = fixture((MAX_INSTANCE_INDEX_ITEMS + 1) as u64, false);
    let snapshot = document.current();
    let query = ModelQuery::default();
    let before = snapshot.canonical_digest();
    let undo = document.visible_undo_steps();

    assert_eq!(
        snapshot.scene_query_bounded(
            MAX_INSTANCE_INDEX_ITEMS,
            MAX_INSTANCE_PATH_STEPS,
            MAX_INSTANCE_INDEX_TEXT_BYTES,
        ),
        Err(SceneQueryBudgetExceeded {
            kind: SceneQueryBudgetKind::Occurrences,
            limit: MAX_INSTANCE_INDEX_ITEMS,
            observed_at_least: MAX_INSTANCE_INDEX_ITEMS + 1,
        })
    );
    let summary = query.summary(&snapshot);
    bounded(&summary);
    assert_eq!(
        summary["counts"]["root_occurrences"],
        MAX_INSTANCE_INDEX_ITEMS + 1
    );
    assert!(summary["counts"]["instances"].is_null());
    assert_eq!(summary["complete"], false);
    assert_eq!(summary["resource_budget"]["status"], "exceeded");
    assert_eq!(summary["resource_budget"]["reason"], "occurrence_count");

    let page = query
        .page(&snapshot, &request(EntityKind::Instances))
        .unwrap();
    bounded(&page);
    assert_eq!(page["items"], json!([]));
    assert!(page["total_matches"].is_null());
    assert_eq!(page["total_matches_complete"], false);
    assert_eq!(page["complete"], false);
    assert_eq!(page["page_complete"], false);
    assert!(page["next_cursor"].is_null());
    assert_eq!(page["resource_budget"]["status"], "exceeded");
    assert_eq!(page["resource_budget"]["limit"], MAX_INSTANCE_INDEX_ITEMS);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo);
}

#[test]
fn instance_index_text_amplification_is_rejected_before_name_clone() {
    let mut document = fixture(1, false);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: DefinitionId(1),
                name: "x".repeat(MAX_INSTANCE_INDEX_TEXT_BYTES + 1),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let exceeded = snapshot
        .scene_query_bounded(
            MAX_INSTANCE_INDEX_ITEMS,
            MAX_INSTANCE_PATH_STEPS,
            MAX_INSTANCE_INDEX_TEXT_BYTES,
        )
        .unwrap_err();
    assert_eq!(exceeded.kind, SceneQueryBudgetKind::TextBytes);
    assert_eq!(exceeded.limit, MAX_INSTANCE_INDEX_TEXT_BYTES);
    assert!(exceeded.observed_at_least > MAX_INSTANCE_INDEX_TEXT_BYTES);

    let query = ModelQuery::default();
    let summary = query.summary(&snapshot);
    bounded(&summary);
    assert_eq!(summary["complete"], false);
    assert_eq!(summary["resource_budget"]["reason"], "text_bytes");
    let page = query
        .page(&snapshot, &request(EntityKind::Instances))
        .unwrap();
    bounded(&page);
    assert_eq!(page["items"], json!([]));
    assert_eq!(page["complete"], false);
    assert_eq!(page["resource_budget"]["reason"], "text_bytes");
}

#[test]
fn bounded_scene_query_caps_root_group_ancestry() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Part".into(),
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "Outer".into(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(2),
                name: "Inner".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(1)),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Nested root".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(2)),
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    assert_eq!(
        document.current().scene_query_bounded(1, 1, 1024),
        Err(SceneQueryBudgetExceeded {
            kind: SceneQueryBudgetKind::PathSteps,
            limit: 1,
            observed_at_least: 2,
        })
    );
}

#[test]
fn filters_catalogs_and_missing_details_are_explicit() {
    let document = fixture(150, false);
    let snapshot = document.current();
    let query = ModelQuery::default();
    let mut req = request(EntityKind::Occurrences);
    req.search = "part-001".into();
    req.definition_id = Some(1);
    assert_eq!(
        collect(&query, &snapshot, req.clone()),
        (100..=150).collect::<Vec<_>>()
    );
    req.definition_id = Some(2);
    assert!(collect(&query, &snapshot, req).is_empty());
    for kind in [EntityKind::Definitions, EntityKind::Features] {
        let mut req = request(kind);
        req.search = if kind == EntityKind::Definitions {
            "component"
        } else {
            "profile"
        }
        .into();
        assert_eq!(collect(&query, &snapshot, req), vec![1]);
        bounded(&query.detail(&snapshot, kind, 1).unwrap());
    }
    assert_eq!(
        query.detail(&snapshot, EntityKind::Features, 1).unwrap()["item"]["kind"],
        "Profile"
    );
    assert_eq!(
        query.detail(&snapshot, EntityKind::Occurrences, 999),
        Err(QueryError::NotFound)
    );
    assert_eq!(
        query.detail(&snapshot, EntityKind::Occurrences, 0),
        Err(QueryError::InvalidInput)
    );
}

#[test]
fn invalid_queries_and_forged_or_cross_query_cursors_fail_closed() {
    let document = fixture(150, false);
    let snapshot = document.current();
    let query = ModelQuery::default();
    let base = request(EntityKind::Occurrences);
    for limit in [0, 101, usize::MAX] {
        let mut req = base.clone();
        req.limit = limit;
        assert_eq!(query.page(&snapshot, &req), Err(QueryError::InvalidInput));
    }
    for (kind, definition_id, search) in [
        (EntityKind::Occurrences, Some(0), String::new()),
        (EntityKind::Definitions, Some(1), String::new()),
        (EntityKind::Occurrences, None, "x".repeat(129)),
    ] {
        let mut req = base.clone();
        req.kind = kind;
        req.definition_id = definition_id;
        req.search = search;
        assert_eq!(query.page(&snapshot, &req), Err(QueryError::InvalidInput));
    }
    for token in [String::new(), "garbage".into(), "x".repeat(4097)] {
        let mut req = base.clone();
        req.cursor = Some(token);
        assert_eq!(query.page(&snapshot, &req), Err(QueryError::InvalidCursor));
    }
    let token = query.page(&snapshot, &base).unwrap()["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    for field in ["kind", "limit", "search", "definition"] {
        let mut req = base.clone();
        req.cursor = Some(token.clone());
        match field {
            "kind" => req.kind = EntityKind::Features,
            "limit" => req.limit = 99,
            "search" => req.search = "part".into(),
            _ => req.definition_id = Some(1),
        }
        assert_eq!(
            query.page(&snapshot, &req),
            Err(QueryError::CrossQueryCursor)
        );
    }
    let mut req = base;
    req.cursor = Some(format!("{token} "));
    assert_eq!(query.page(&snapshot, &req), Err(QueryError::InvalidCursor));
    req.cursor = Some(token);
    assert_eq!(
        ModelQuery::default().page(&snapshot, &req),
        Err(QueryError::InvalidCursor)
    );
    for raw in [
        r#"{"kind":"occurrences","limit":-1}"#,
        r#"{"kind":"occurrences","extra":true}"#,
        r#"{"kind":"spatial"}"#,
        r#"{"kind":"features","limit":1.5}"#,
    ] {
        assert!(serde_json::from_str::<PageRequest>(raw).is_err());
    }
}

#[test]
fn cursor_is_bound_to_document_revision_digest_and_mutation_generation() {
    let mut document = fixture(150, false);
    let mut query = ModelQuery::default();
    let original = document.current();
    let mut req = request(EntityKind::Occurrences);
    req.cursor = Some(
        query.page(&original, &req).unwrap()["next_cursor"]
            .as_str()
            .unwrap()
            .into(),
    );
    assert_eq!(
        query.page(&fixture(150, false).current(), &req),
        Err(QueryError::StaleCursor)
    );
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameEntity {
            id: OccurrenceId(1),
            name: "renamed".into(),
        }]))
        .unwrap();
    assert_eq!(
        query.page(&document.current(), &req),
        Err(QueryError::StaleCursor)
    );
    query.invalidate(); // Required host hook, including mutations followed by Undo.
    document.undo().unwrap();
    query.invalidate();
    assert_eq!(document.current().revision_id(), original.revision_id());
    assert_eq!(
        document.current().canonical_digest(),
        original.canonical_digest()
    );
    assert_eq!(
        query.page(&document.current(), &req),
        Err(QueryError::StaleCursor)
    );
    document.redo().unwrap();
    query.invalidate();
    assert_eq!(
        query.page(&document.current(), &req),
        Err(QueryError::StaleCursor)
    );
}

#[test]
fn escaped_names_byte_limited_pages_unicode_and_receipts_remain_bounded() {
    let document = fixture(250, true);
    let snapshot = document.current();
    let query = ModelQuery::default();
    let req = request(EntityKind::Occurrences);
    let page = query.page(&snapshot, &req).unwrap();
    bounded(&page);
    assert_eq!(page["byte_limited"], true);
    assert_eq!(page["items"][0]["name"]["truncated"], true);
    assert_eq!(
        collect(&query, &snapshot, req),
        (1..=250).collect::<Vec<_>>()
    );
    let detail = query.detail(&snapshot, EntityKind::Occurrences, 1).unwrap();
    bounded(&detail);
    assert_eq!(detail["completeness"]["metadata_only"], true);
    let unicode = bounded_text(&"€".repeat(1000));
    assert_eq!(unicode["text"].as_str().unwrap().len(), 126);
    assert_eq!(unicode["original_bytes"], 3000);
    let receipt = created_receipt(&DocumentStore::new().current(), &snapshot);
    bounded(&receipt);
    assert_eq!(receipt["occurrence_ids"]["total"], 250);
    assert_eq!(
        receipt["occurrence_ids"]["ids"].as_array().unwrap().len(),
        100
    );
    assert_eq!(receipt["occurrence_ids"]["complete"], false);
    assert_eq!(receipt["feature_ids"]["complete"], true);
}

#[test]
fn occurrence_properties_are_bounded_and_detail_stays_within_output_budget() {
    let mut document = fixture(1, false);
    let mut commands = Vec::new();
    for id in 1..=100 {
        commands.push(CanonicalCommand::UpsertClassificationDimension {
            id: ClassificationDimensionId(id),
            name: format!("{}-{id}", "dimension".repeat(20)),
            categories: vec![(
                ClassificationCategoryId(id),
                format!("{}-{id}", "category".repeat(20)),
            )],
        });
        commands.push(CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(1),
            dimension_id: ClassificationDimensionId(id),
            category_id: Some(ClassificationCategoryId(id)),
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let snapshot = document.current();
    let query = ModelQuery::default();
    let page = query
        .page(&snapshot, &request(EntityKind::Occurrences))
        .unwrap();
    bounded(&page);
    assert_eq!(
        page["items"][0]["properties"]["classifications"]["total"],
        100
    );
    assert_eq!(
        page["items"][0]["properties"]["classifications"]["items"]
            .as_array()
            .unwrap()
            .len(),
        32
    );
    assert_eq!(
        page["items"][0]["properties"]["classifications"]["complete"],
        false
    );
    bounded(&query.detail(&snapshot, EntityKind::Occurrences, 1).unwrap());
}

#[test]
fn nested_instance_pages_return_qualified_paths_and_fail_stale_without_mutation() {
    let mut document = nested_fixture();
    let snapshot = document.current();
    let mut query = ModelQuery::default();
    assert_eq!(
        snapshot.scene_query_bounded(MAX_INSTANCE_INDEX_ITEMS, 0, MAX_INSTANCE_INDEX_TEXT_BYTES),
        Err(SceneQueryBudgetExceeded {
            kind: SceneQueryBudgetKind::PathSteps,
            limit: 0,
            observed_at_least: 1,
        })
    );
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    let mut req = request(EntityKind::Instances);
    req.limit = 1;
    let first = query.page(&snapshot, &req).unwrap();
    bounded(&first);
    assert_eq!(first["coverage"]["nested_hierarchy"], true);
    let old_cursor = first["next_cursor"].as_str().unwrap().to_owned();
    let mut alternate = request(EntityKind::Instances);
    alternate.search = "copy".into();
    assert_eq!(
        query.page(&snapshot, &alternate).unwrap()["total_matches"],
        1
    );
    let mut resumed = request(EntityKind::Instances);
    resumed.limit = 1;
    resumed.cursor = Some(old_cursor.clone());
    assert_eq!(
        query.page(&snapshot, &resumed).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut items = Vec::new();
    loop {
        let page = query.page(&snapshot, &req).unwrap();
        bounded(&page);
        assert_eq!(page["total_matches"], 4);
        items.extend(page["items"].as_array().unwrap().iter().cloned());
        if page["complete"] == true {
            break;
        }
        req.cursor = Some(page["next_cursor"].as_str().unwrap().to_owned());
    }
    assert_eq!(items.len(), 4);
    let mut ids = items
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4);

    let nested = items
        .iter()
        .filter(|item| item["instance_depth"] == 1)
        .collect::<Vec<_>>();
    assert_eq!(nested.len(), 2);
    assert_ne!(nested[0]["id"], nested[1]["id"]);
    for item in &nested {
        assert_eq!(item["definition_id"], 1);
        assert_eq!(item["instance_path"]["steps"].as_array().unwrap().len(), 1);
        assert_eq!(item["instance_path"]["steps"][0]["owner_definition_id"], 2);
        assert_eq!(item["instance_path"]["steps"][0]["kind"], "occurrence");
        assert_eq!(item["instance_path"]["steps"][0]["local_id"], 1);
        assert!(item["parent_path"].is_object());
    }
    assert_ne!(nested[0]["world_transform"], nested[1]["world_transform"]);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    let root_id = snapshot.occurrences().next().unwrap().id();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameEntity {
            id: root_id,
            name: "Renamed assembly".into(),
        }]))
        .unwrap();
    query.invalidate();
    let mut stale = request(EntityKind::Instances);
    stale.limit = 1;
    stale.cursor = Some(old_cursor);
    assert_eq!(
        query.page(&document.current(), &stale),
        Err(QueryError::StaleCursor)
    );
    document.undo().unwrap();
    query.invalidate();
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(
        query.page(&document.current(), &stale),
        Err(QueryError::StaleCursor)
    );
}

#[test]
fn instance_property_filters_preserve_local_source_and_root_classification_scope() {
    let document = nested_fixture();
    let snapshot = document.current();
    let query = ModelQuery::default();

    let mut by_tag = request(EntityKind::Instances);
    by_tag.limit = 1;
    by_tag.tag_id = Some(7);
    let tag_ids = collect_instance_ids(&query, &snapshot, by_tag.clone());
    assert_eq!(tag_ids.len(), 2);
    assert_ne!(tag_ids[0], tag_ids[1]);
    let first = query.page(&snapshot, &by_tag).unwrap();
    let properties = &first["items"][0]["properties"];
    assert_eq!(properties["tag_match_scope"], "any_occurrence_step");
    assert_eq!(properties["tags"][0]["id"], 7);
    assert_eq!(properties["tags"][0]["source"]["kind"], "local_occurrence");
    assert_eq!(properties["tags"][0]["source"]["owner_definition_id"], 2);
    assert_eq!(properties["tags"][0]["source"]["local_id"], 1);

    let mut primary = request(EntityKind::Instances);
    primary.classification_dimension_id = Some(9);
    primary.classification_category_id = Some(10);
    let primary_page = query.page(&snapshot, &primary).unwrap();
    assert_eq!(primary_page["total_matches"], 2);
    assert!(
        primary_page["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| {
                item["properties"]["classification_scope"] == "root_occurrence"
                    && item["properties"]["classifications"]["items"][0]["category_id"] == 10
            })
    );

    let mut root_primary = request(EntityKind::Occurrences);
    root_primary.classification_dimension_id = Some(9);
    root_primary.classification_category_id = Some(10);
    assert_eq!(collect(&query, &snapshot, root_primary), vec![2]);

    let mut invalid = request(EntityKind::Instances);
    invalid.classification_category_id = Some(10);
    assert_eq!(
        query.page(&snapshot, &invalid),
        Err(QueryError::InvalidInput)
    );
    let mut forbidden = request(EntityKind::Features);
    forbidden.tag_id = Some(7);
    assert_eq!(
        query.page(&snapshot, &forbidden),
        Err(QueryError::InvalidInput)
    );

    let first_page = query.page(&snapshot, &by_tag).unwrap();
    let mut crossed = by_tag;
    crossed.cursor = first_page["next_cursor"].as_str().map(str::to_owned);
    crossed.tag_id = None;
    assert_eq!(
        query.page(&snapshot, &crossed),
        Err(QueryError::CrossQueryCursor)
    );
}

#[test]
fn spatial_bounds_use_revision_bound_bvh_and_report_incomplete_proxy_coverage() {
    let mut document = spatial_fixture();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: TagId(7),
                name: "Bounded candidates".into(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(TagId(7)),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let query = ModelQuery::default();
    let mut req = request(EntityKind::Instances);
    req.world_bounds_mm = Some([[-1.0, -1.0, -1.0], [20.0, 20.0, 20.0]]);
    let page = query.page(&snapshot, &req).unwrap();
    bounded(&page);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["id"], "root:1");
    assert_eq!(
        page["items"][0]["world_bounds"]["min_mm"],
        json!([0.0, 0.0, 0.0])
    );
    assert_eq!(
        page["items"][0]["world_bounds"]["max_mm"],
        json!([10.0, 10.0, 10.0])
    );
    assert_eq!(
        page["items"][0]["world_bounds"]["origin"],
        json!({"kind":"canonical_profile_extrusion_proxy","profile_feature_id":1,
            "extrusion_feature_id":2})
    );
    assert_eq!(page["coverage"]["spatial"], true);
    assert_eq!(page["coverage"]["spatial_candidates_complete"], false);
    assert_eq!(page["page_complete"], true);
    assert_eq!(page["complete"], false);
    assert_eq!(page["total_matches_complete"], false);
    assert!(page["next_cursor"].is_null());
    assert_eq!(page["spatial_query"]["coordinate_space"], "world_mm");
    assert_eq!(page["spatial_query"]["predicate"], "intersects");
    assert_eq!(page["spatial_query"]["indexed_instances"], 2);
    assert_eq!(page["spatial_query"]["unbounded_scope_instances"], 1);
    assert_eq!(page["spatial_query"]["candidates_complete"], false);
    assert!(page["spatial_query"]["bounds_tested"].as_u64().unwrap() < 10);

    req.definition_id = Some(1);
    let bounded_scope = query.page(&snapshot, &req).unwrap();
    assert_eq!(
        bounded_scope["spatial_query"]["unbounded_scope_instances"],
        0
    );
    assert_eq!(
        bounded_scope["coverage"]["spatial_candidates_complete"],
        true
    );
    assert_eq!(bounded_scope["complete"], true);
    assert_eq!(bounded_scope["total_matches_complete"], true);

    req.definition_id = None;
    req.tag_id = Some(7);
    let property_scope = query.page(&snapshot, &req).unwrap();
    assert_eq!(property_scope["total_matches"], 1);
    assert_eq!(
        property_scope["spatial_query"]["unbounded_scope_instances"],
        0
    );
    assert_eq!(property_scope["complete"], true);

    let all = query
        .page(&snapshot, &request(EntityKind::Instances))
        .unwrap();
    assert!(
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "root:3" && item["world_bounds"].is_null())
    );
}

#[test]
fn spatial_bounds_validation_and_cursor_scope_fail_closed() {
    let document = spatial_fixture();
    let snapshot = document.current();
    let query = ModelQuery::default();
    for bounds in [
        [[1.0, 0.0, 0.0], [0.0, 1.0, 1.0]],
        [[f64::NAN, 0.0, 0.0], [1.0, 1.0, 1.0]],
        [[0.0, 0.0, 0.0], [f64::INFINITY, 1.0, 1.0]],
    ] {
        let mut req = request(EntityKind::Instances);
        req.world_bounds_mm = Some(bounds);
        assert_eq!(query.page(&snapshot, &req), Err(QueryError::InvalidInput));
    }
    let mut wrong_kind = request(EntityKind::Occurrences);
    wrong_kind.world_bounds_mm = Some([[0.0; 3], [1.0; 3]]);
    assert_eq!(
        query.page(&snapshot, &wrong_kind),
        Err(QueryError::InvalidInput)
    );

    let mut req = request(EntityKind::Instances);
    req.limit = 1;
    req.world_bounds_mm = Some([[-1.0; 3], [200.0; 3]]);
    let token = query.page(&snapshot, &req).unwrap()["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    req.cursor = Some(token);
    req.world_bounds_mm = Some([[-1.0; 3], [20.0; 3]]);
    assert_eq!(
        query.page(&snapshot, &req),
        Err(QueryError::CrossQueryCursor)
    );
}

#[test]
fn relation_queries_stream_canonical_hierarchy_definition_and_assembly_edges() {
    let mut hierarchy = DocumentStore::new();
    hierarchy
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Shared part".into(),
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "Outer".into(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(2),
                name: "Inner".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(1)),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "First shared use".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(2)),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Second shared use".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let snapshot = hierarchy.current();
    let mut query = ModelQuery::default();
    assert_eq!(query.summary(&snapshot)["counts"]["relations"], 4);
    let relation_workset = query
        .create_workset(&snapshot, &request(EntityKind::Relations))
        .unwrap();
    assert_eq!(relation_workset["scope"]["entity_kind"], "relations");
    assert_eq!(relation_workset["item_count"], 4);
    assert_eq!(relation_workset["completeness"]["usable_for_batch"], true);

    let mut paged_request = request(EntityKind::Relations);
    paged_request.limit = 1;
    let first = query.page(&snapshot, &paged_request).unwrap();
    assert_eq!(first["items"][0]["relation_type"], "uses_definition");
    assert_eq!(first["items"][0]["direction"], "outgoing");
    assert_eq!(
        first["items"][0]["source"],
        json!({"kind":"occurrence","id":1})
    );
    assert_eq!(
        first["items"][0]["target"],
        json!({"kind":"definition","id":1})
    );
    assert_eq!(first["total_matches"], 4);
    assert_eq!(first["total_matches_complete"], true);
    assert_eq!(first["page_complete"], false);
    let first_cursor = first["next_cursor"].as_str().unwrap().to_owned();

    let mut ids = Vec::new();
    loop {
        let page = query.page(&snapshot, &paged_request).unwrap();
        bounded(&page);
        ids.extend(
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned()),
        );
        if page["complete"] == true {
            break;
        }
        paged_request.cursor = Some(page["next_cursor"].as_str().unwrap().to_owned());
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4);

    let mut members = request(EntityKind::Relations);
    members.search = "member_of_group".into();
    let member_page = query.page(&snapshot, &members).unwrap();
    assert_eq!(member_page["total_matches"], 2);
    assert!(member_page["items"].as_array().unwrap().iter().all(|item| {
        item["relation_type"] == "member_of_group" && item["direction"] == "outgoing"
    }));

    let mut shared = request(EntityKind::Relations);
    shared.definition_id = Some(1);
    assert_eq!(query.page(&snapshot, &shared).unwrap()["total_matches"], 3);

    let mut crossed = request(EntityKind::Relations);
    crossed.limit = 1;
    crossed.cursor = Some(first_cursor.clone());
    crossed.search = "member".into();
    assert_eq!(
        query.page(&snapshot, &crossed),
        Err(QueryError::CrossQueryCursor)
    );

    hierarchy
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameEntity {
            id: OccurrenceId(1),
            name: "Renamed".into(),
        }]))
        .unwrap();
    query.invalidate();
    let mut stale = request(EntityKind::Relations);
    stale.limit = 1;
    stale.cursor = Some(first_cursor);
    assert_eq!(
        query.page(&hierarchy.current(), &stale),
        Err(QueryError::StaleCursor)
    );

    let nested = nested_fixture().current();
    let mut nested_definition = request(EntityKind::Relations);
    nested_definition.definition_id = Some(1);
    let local_relation = ModelQuery::default()
        .page(&nested, &nested_definition)
        .unwrap();
    assert_eq!(local_relation["total_matches"], 1);
    assert_eq!(
        local_relation["items"][0]["relation_type"],
        "uses_definition"
    );
    assert_eq!(
        local_relation["items"][0]["source"]["kind"],
        "local_occurrence"
    );
    assert_eq!(
        local_relation["items"][0]["source"]["owner_definition_id"],
        2
    );
    assert_eq!(local_relation["items"][0]["source"]["local_id"], 1);
    assert_eq!(
        local_relation["items"][0]["target"],
        json!({"kind":"definition","id":1})
    );

    let mut assembly = fixture(2, false);
    let reference = planar_reference(&assembly.current());
    let endpoint = |occurrence_id| {
        AssemblyMateEndpoint::resolved_planar_face(
            occurrence_id,
            PlanarFaceAttachment::new(reference.clone(), [0.0; 3], [0.0, 0.0, 1.0]).unwrap(),
        )
    };
    assembly
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AssemblyMateId(7),
                endpoint(OccurrenceId(1)),
                endpoint(OccurrenceId(2)),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
        ]))
        .unwrap();
    let mut mates = request(EntityKind::Relations);
    mates.search = "assembly_mate".into();
    let mate_page = ModelQuery::default()
        .page(&assembly.current(), &mates)
        .unwrap();
    assert_eq!(mate_page["total_matches"], 1);
    assert_eq!(mate_page["items"][0]["id"], "assembly_mate:7");
    assert_eq!(mate_page["items"][0]["direction"], "bidirectional");
    assert_eq!(mate_page["items"][0]["source"]["id"], 1);
    assert_eq!(mate_page["items"][0]["target"]["id"], 2);
    assert_eq!(
        mate_page["items"][0]["source"]["reference_health"]["status"],
        "resolved"
    );
    assert_eq!(mate_page["items"][0]["mate"]["kind"], "coincident_planar");
    assert_eq!(
        mate_page["items"][0]["origin"]["kind"],
        "canonical_assembly_mate"
    );
}

#[test]
fn empty_snapshot_has_complete_empty_pages() {
    let snapshot = DocumentStore::new().current();
    let query = ModelQuery::default();
    for kind in [
        EntityKind::Occurrences,
        EntityKind::Instances,
        EntityKind::Definitions,
        EntityKind::Features,
        EntityKind::Relations,
    ] {
        let page = query.page(&snapshot, &request(kind)).unwrap();
        bounded(&page);
        assert_eq!(page["total_matches"], 0);
        assert_eq!(page["complete"], true);
        assert_eq!(page["items"], json!([]));
    }
}
