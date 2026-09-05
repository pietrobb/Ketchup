use ketchup_application::model_query::*;
use ketchup_core::document::*;
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
fn request(kind: EntityKind) -> PageRequest {
    PageRequest {
        kind,
        limit: 100,
        search: String::new(),
        definition_id: None,
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
        json!({"root_occurrences":10000,"definitions":1,"features":1})
    );
    assert_eq!(summary["coverage"]["nested_hierarchy"], false);
    assert_eq!(
        collect(&query, &snapshot, request(EntityKind::Occurrences)),
        (1..=10_000).collect::<Vec<_>>()
    );
    assert_eq!(query.summary(&snapshot), summary);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo);
    assert_eq!(document.visible_redo_steps(), 0);
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
fn empty_snapshot_has_complete_empty_pages() {
    let snapshot = DocumentStore::new().current();
    let query = ModelQuery::default();
    for kind in [
        EntityKind::Occurrences,
        EntityKind::Definitions,
        EntityKind::Features,
    ] {
        let page = query.page(&snapshot, &request(kind)).unwrap();
        bounded(&page);
        assert_eq!(page["total_matches"], 0);
        assert_eq!(page["complete"], true);
        assert_eq!(page["items"], json!([]));
    }
}
