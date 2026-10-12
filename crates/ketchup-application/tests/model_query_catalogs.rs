use ketchup_application::model_query::*;
use ketchup_geometry::prismatic::{Aabb, CanonicalJoint, JointId};
use ketchup_model::document::*;
use ketchup_model::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceOwner, ClearanceSeverity, ClearanceVolumeId,
    SpaceId,
};
use ketchup_model::tolerance::TolerancePolicy;
use serde_json::{Value, json};

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
        compact: false,
    }
}

fn product_entities() -> DocumentStore {
    let aabb = |min, max| Aabb::new(min, max).unwrap();
    let rule_output = |key: &str| {
        DerivedIdentity::new(
            NodeId(4),
            SlotPath::new(vec![SlotSegment::new(NodeId(4), "result", key).unwrap()]).unwrap(),
        )
        .unwrap()
    };
    let height = |feature| {
        PersistentDimensionTarget::FeatureParameter(
            FeatureParameterTarget::new(feature, "extent.distance", ParameterValueType::Length)
                .unwrap(),
        )
    };
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Cabinet".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Rectangle".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Extrusion".into(),
                kind: FeatureKind::extrusion(FeatureId(1), Dimension::from_decimal("25").unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Cabinet 1".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::UpsertSpace(
                CanonicalSpace::new(
                    SpaceId(70),
                    "Kitchen",
                    aabb([0.0, 0.0, 0.0], [3000.0, 2000.0, 2500.0]),
                    vec![SpaceId(75)],
                    Vec::new(),
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertSpace(
                CanonicalSpace::new(
                    SpaceId(75),
                    "Pantry",
                    aabb([3000.0, 0.0, 0.0], [4000.0, 1000.0, 2500.0]),
                    vec![SpaceId(70)],
                    vec![SpaceId(70)],
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertClearanceVolume(
                CanonicalClearanceVolume::new(
                    ClearanceVolumeId(71),
                    ClearanceOwner::Occurrence(InstancePath::root(OccurrenceId(1))),
                    "Door swing",
                    aabb([0.0, -600.0, 0.0], [600.0, 0.0, 2000.0]),
                    TolerancePolicy::new(0.5).unwrap(),
                    ClearanceSeverity::Required,
                    None,
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertClearanceVolume(
                CanonicalClearanceVolume::new(
                    ClearanceVolumeId(72),
                    ClearanceOwner::Space(SpaceId(70)),
                    "Walkway",
                    aabb([0.0, 0.0, 0.0], [900.0, 2000.0, 2000.0]),
                    TolerancePolicy::new(1.0).unwrap(),
                    ClearanceSeverity::Advisory,
                    None,
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertJoint(
                CanonicalJoint::new(
                    JointId(98),
                    rule_output("left").into(),
                    rule_output("right").into(),
                    aabb([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]),
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertPersistentDimension(
                PersistentDimension::new(
                    PersistentDimensionId(90),
                    "Cabinet height",
                    height(FeatureId(2)),
                    DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 1).unwrap(),
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertPersistentDimension(
                PersistentDimension::new(
                    PersistentDimensionId(91),
                    "Removed shelf",
                    height(FeatureId(99)),
                    DimensionPresentation::new(DimensionDisplayUnit::Millimetres, 0).unwrap(),
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    document
}

fn items(query: &ModelQuery, snapshot: &Snapshot, kind: EntityKind) -> Vec<Value> {
    let page = query.page(snapshot, &request(kind)).unwrap();
    assert_eq!(page["complete"], true, "{page}");
    page["items"].as_array().unwrap().clone()
}

#[test]
fn spaces_clearances_joints_and_dimensions_are_listed_with_their_stored_fields() {
    let document = product_entities();
    let snapshot = document.current();
    let query = ModelQuery::default();

    let counts = &query.summary(&snapshot)["counts"];
    assert_eq!(
        [
            &counts["spaces"],
            &counts["clearance_volumes"],
            &counts["joints"],
            &counts["dimensions"]
        ],
        [&json!(2), &json!(2), &json!(1), &json!(2)]
    );

    let spaces = items(&query, &snapshot, EntityKind::Spaces);
    assert_eq!(
        spaces[0],
        json!({"id":70,"purpose":{"text":"Kitchen","original_bytes":7,"truncated":false},
            "volume":{"min_mm":[0.0,0.0,0.0],"max_mm":[3000.0,2000.0,2500.0],
                "coordinate_space":"world_mm"},
            "adjacent_to":[75],"accessible_to":[]})
    );
    assert_eq!(spaces[1]["accessible_to"], json!([70]));

    let clearances = items(&query, &snapshot, EntityKind::ClearanceVolumes);
    assert_eq!(
        clearances[0]["owner"],
        json!({"kind":"instance","root_occurrence_id":1,"resolved":true,
            "instance_path":{"root_occurrence_id":1,"steps":[]}})
    );
    assert_eq!(clearances[0]["reason"]["text"], "Door swing");
    assert_eq!(clearances[0]["severity"], "required");
    assert_eq!(clearances[0]["tolerance_mm"], 0.5);
    assert_eq!(
        clearances[1]["owner"],
        json!({"kind":"space","space_id":70,"resolved":true})
    );
    assert_eq!(clearances[1]["severity"], "advisory");

    let joints = items(&query, &snapshot, EntityKind::Joints);
    assert_eq!(joints.len(), 1);
    assert_eq!(joints[0]["id"], 98);
    assert_eq!(
        joints[0]["participant_a"],
        json!({"kind":"derived","root_rule_node_id":4,
            "slot_path":[{"producer_rule_id":4,"output_port":"result","semantic_key":"left"}],
            "resolution":{"status":"lost"}})
    );
    assert_eq!(
        joints[0]["participant_b"]["slot_path"][0]["semantic_key"],
        "right"
    );
    assert_eq!(joints[0]["volume"]["max_mm"], json!([4.0, 5.0, 6.0]));

    let dimensions = items(&query, &snapshot, EntityKind::Dimensions);
    assert_eq!(
        dimensions[0],
        json!({"id":90,"name":{"text":"Cabinet height","original_bytes":14,"truncated":false},
            "target":{"kind":"feature_parameter","feature_id":2,"path":"extent.distance",
                "value_type":"length"},
            "presentation":{"unit":"cm","decimal_places":1},
            "status":"resolved","ambiguous_segment_index":null,
            "value_mm":25.0,"display_value":2.5,"display_text":"2.5 cm"})
    );
    assert_eq!(dimensions[1]["status"], "lost");
    assert_eq!(dimensions[1]["value_mm"], Value::Null);
    assert_eq!(dimensions[1]["display_text"], Value::Null);

    let mut search = request(EntityKind::Spaces);
    search.search = "Pan".into();
    let found = query.page(&snapshot, &search).unwrap();
    assert_eq!(found["items"][0]["id"], 75);
    assert_eq!(found["total_matches"], 1);

    let detail = query.detail(&snapshot, EntityKind::Dimensions, 90).unwrap();
    assert_eq!(detail["item"], dimensions[0]);
    assert_eq!(
        query.detail(&snapshot, EntityKind::Joints, 99),
        Err(QueryError::NotFound)
    );
    let mut by_definition = request(EntityKind::Joints);
    by_definition.definition_id = Some(1);
    assert_eq!(
        query.page(&snapshot, &by_definition),
        Err(QueryError::InvalidInput)
    );
}

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
            kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]]),
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let query = ModelQuery::default();
    let snapshot = document.current();
    for kind in [EntityKind::Definitions, EntityKind::Features] {
        let mut request = request(kind);
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
