use ketchup_application::model_query::*;
use ketchup_core::document::*;

#[test]
fn definition_and_feature_catalogs_page_and_definition_detail_caps_feature_ids() {
    let mut document = DocumentStore::new();
    let mut commands = Vec::new();
    for id in 1..=151 {
        commands.push(CanonicalCommand::CreateDefinition {
            id: DefinitionId(id),
            name: format!("definition-{id}"),
        });
        commands.push(CanonicalCommand::CreateFeature {
            id: FeatureId(id),
            definition_id: DefinitionId(1),
            name: format!("profile-{id}"),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
            },
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let query = ModelQuery::default();
    let snapshot = document.current();
    for kind in [EntityKind::Definitions, EntityKind::Features] {
        let mut request = PageRequest {
            kind,
            limit: 100,
            search: String::new(),
            definition_id: None,
            cursor: None,
        };
        let mut ids = Vec::new();
        loop {
            let page = query.page(&snapshot, &request).unwrap();
            assert!(serde_json::to_vec(&page).unwrap().len() < MAX_OUTPUT_BYTES);
            assert_eq!(page["total_matches"], 151);
            ids.extend(
                page["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["id"].as_u64().unwrap()),
            );
            if page["complete"] == true {
                break;
            }
            request.cursor = Some(page["next_cursor"].as_str().unwrap().into());
        }
        assert_eq!(ids, (1..=151).collect::<Vec<_>>());
    }
    let detail = query.detail(&snapshot, EntityKind::Definitions, 1).unwrap();
    assert!(serde_json::to_vec(&detail).unwrap().len() < MAX_OUTPUT_BYTES);
    assert_eq!(detail["item"]["feature_ids"]["total"], 151);
    assert_eq!(detail["item"]["feature_ids"]["complete"], false);
    assert_eq!(
        detail["item"]["feature_ids"]["ids"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
}
