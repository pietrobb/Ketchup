//! The public tool contract retains exact face references and the document guard.
use super::*;

#[test]
fn measurement_schema_and_transport_preserve_both_modes_and_nested_paths() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let schemas = crate::schema::tools();
    let schema = &schemas
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "inspect")
        .unwrap()["inputSchema"];
    assert_eq!(
        schema["allOf"][0]["then"]["required"],
        json!(["expected", "faces", "mode"])
    );
    assert_eq!(
        schema["allOf"][1]["if"]["properties"]["mode"]["const"],
        "supporting_planes"
    );
    assert_eq!(schema["allOf"][1]["then"]["required"], json!(["direction"]));
    assert_eq!(schema["properties"]["faces"]["minItems"], 2);
    assert_eq!(schema["properties"]["faces"]["maxItems"], 2);
    let stamp = json!({"document_id":1,"revision":7,"canonical_digest":"d","mutation_epoch":0});
    let faces = json!([
        {"entity_id":11,"instance_path":{"root_occurrence_id":4,"steps":[]}},
        {"entity_id":12,"instance_path":{"root_occurrence_id":8,"steps":[{"kind":"occurrence","owner_definition_id":5,"local_id":2}]}}
    ]);
    for mode in ["minimum", "supporting_planes"] {
        let mut arguments = json!({"action":"measure","mode":mode,"faces":faces,"expected":stamp});
        if mode == "supporting_planes" {
            arguments["direction"] = json!([0, 0, 1]);
        }
        let result = call(&mut tools, "inspect", arguments.clone());
        assert_eq!(result["isError"], false, "{result}");
        let mut expected = arguments;
        expected.as_object_mut().unwrap().remove("action");
        expected["method"] = json!("measure_faces");
        assert_eq!(received.recv().unwrap(), expected);
        assert_eq!(text(&result)["stamp"], stamp);
    }
    let result = call(
        &mut tools,
        "program",
        json!({"action":"validate", "expected":stamp, "validators":["beam_deflection"], "motion":{"name":"travel","from":0,"to":20}}),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(
        received.recv().unwrap(),
        json!({"method":"validate_program","expected":stamp,"validators":["beam_deflection"],"motion":{"name":"travel","from":0,"to":20}})
    );
    assert!(
        schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("mass"))
    );
    let result = call(
        &mut tools,
        "inspect",
        json!({"action":"mass", "expected":stamp, "occurrence_ids":[4, 8]}),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(
        received.recv().unwrap(),
        json!({"method":"mass","expected":stamp,"occurrence_ids":[4, 8]})
    );
}

#[test]
fn underlay_snap_reaches_the_window_with_its_point_and_radius() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let schemas = crate::schema::tools();
    let schema = &schemas
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "inspect")
        .unwrap()["inputSchema"];
    assert_eq!(schema["allOf"][2]["then"]["required"], json!(["point_mm"]));
    let arguments =
        json!({"action":"underlay_snap", "point_mm":[120.5, -3], "radius_mm":25, "underlay_id":2});
    let result = call(&mut tools, "inspect", arguments);
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(
        received.recv().unwrap(),
        json!({"method":"underlay_snap", "point_mm":[120.5, -3], "radius_mm":25, "underlay_id":2})
    );
}
