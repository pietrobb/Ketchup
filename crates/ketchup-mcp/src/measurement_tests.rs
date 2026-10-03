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
        json!({"action":"validate", "expected":stamp}),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(
        received.recv().unwrap(),
        json!({"method":"validate_program","expected":stamp})
    );
}
