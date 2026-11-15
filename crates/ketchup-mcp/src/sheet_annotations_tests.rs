use super::*;

#[test]
fn drawing_sheet_annotations_are_discoverable_and_reach_the_window() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let schemas = crate::schema::tools();
    let drawing = schemas
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "drawing")
        .unwrap();
    for action in ["sheet", "annotate"] {
        assert!(
            drawing["inputSchema"]["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!(action))
        );
    }
    assert_eq!(
        call(
            &mut tools,
            "drawing",
            json!({"action": "sheet", "sheet_id": 7, "view": "front", "offset": 10, "limit": 20})
        )["isError"],
        false
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method": "drawing_sheet", "sheet_id": 7, "view": "front", "offset": 10, "limit": 20})
    );
    let annotations = json!([{"kind": "radius", "view": "front", "source_circle_id": "sheet-7/view-front/circle-1", "leader_angle_degrees": 45.0, "offset_page_mm": 6.0}]);
    assert_eq!(
        call(
            &mut tools,
            "drawing",
            json!({"action": "annotate", "sheet_id": 7, "annotations": annotations})
        )["isError"],
        false
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method": "drawing_annotate", "sheet_id": 7, "annotations": annotations})
    );
    assert_eq!(
        drawing["inputSchema"]["properties"]["hole_callouts"]["type"],
        "boolean"
    );
    assert_eq!(
        call(
            &mut tools,
            "drawing",
            json!({"action": "annotate", "sheet_id": 7, "hole_callouts": true})
        )["isError"],
        false
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method": "drawing_annotate", "sheet_id": 7, "hole_callouts": true})
    );
}
