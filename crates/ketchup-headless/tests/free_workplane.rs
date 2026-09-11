use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    state: Value,
}
impl Client {
    fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ketchup-headless"))
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            input,
            output,
            state: Value::Null,
        };
        client.call("state", json!({}), false);
        client
    }
    fn call(&mut self, method: &str, mut params: Value, mutation: bool) -> Value {
        if mutation {
            params["expected_revision"] = self.state["revision"].clone();
            params["expected_digest"] = self.state["canonical_digest"].clone();
        }
        writeln!(
            self.input,
            "{}",
            json!({"protocol":"ketchup.headless.v1","id":1,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        let result = response["result"].clone();
        if result.get("state").is_some() {
            self.state = result["state"].clone();
        }
        result
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn polygon(points: &[[f64; 2]]) -> (Value, Value) {
    let mut entities = Vec::new();
    let mut constraints = Vec::new();
    for (i, start) in points.iter().enumerate() {
        let end = points[(i + 1) % points.len()];
        entities.push(json!({"type":"line","id":i+1,"start_mm":start,"end_mm":end}));
        for (j, (point, position)) in [("start", *start), ("end", end)].into_iter().enumerate() {
            constraints.push(json!({"type":"fixed_point","id":2*i+j+1,"point":{"entity_id":i+1,"point":point},"position_mm":position}));
        }
    }
    (json!(entities), json!(constraints))
}
fn frame(origin: [f64; 3]) -> Value {
    json!({"type":"frame","origin_mm":origin,"x_axis":[0.8,0.6,0.0],"y_axis":[0.0,0.0,1.0]})
}
fn native_volume(result: &Value, feature: u64) -> f64 {
    assert_eq!(result["complete"], true, "{result}");
    let geometry = result["topology_geometry"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["feature_id"] == feature)
        .expect("native topology result");
    geometry["native_evidence"]["volume_mm3"]
        .as_f64()
        .expect("native BRep volume")
}

#[test]
fn native_headless_translated_rotated_frame_wedge_pocket_roundtrips() {
    let mut client = Client::new();
    let capabilities = client.call("capabilities", json!({}), false);
    let variants = capabilities["cad_program_schema"]["$defs"]["AssistantWorkplaneSpec"]["oneOf"]
        .as_array()
        .unwrap();
    let schema = variants
        .iter()
        .find(|v| v["properties"]["type"]["const"] == "frame")
        .unwrap();
    assert_eq!(schema["additionalProperties"], false);
    let mut required = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    required.sort_unstable();
    assert_eq!(required, ["origin_mm", "type", "x_axis", "y_axis"]);
    for axis in ["origin_mm", "x_axis", "y_axis"] {
        assert_eq!(schema["properties"][axis]["minItems"], 3);
        assert_eq!(schema["properties"][axis]["maxItems"], 3);
    }
    let (entities, constraints) = polygon(&[[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]]);
    let created = client.call("apply", json!({"program":{"operations":[{
        "operation":"create_part","name":"Rotated blank","workplane":frame([120.0,-40.0,70.0]),
        "entities":entities,"constraints":constraints,"feature":{"type":"extrusion","distance_mm":10.0},"translation_mm":[0.0,0.0,0.0]
    }]}}), true);
    let definition = created["created"]["definition_ids"][0].as_u64().unwrap();
    let pad = created["created"]["feature_ids"][2].as_u64().unwrap();
    let result = client.call("evaluate", json!({"timeout_ms":30000}), false);
    assert!((native_volume(&result, pad) - 4000.0).abs() < 1e-6);

    // The triangular profile starts six millimetres along the rotated normal;
    // cutting four more removes a wedge of volume 20*20/2*4 = 800 mm^3.
    let (entities, constraints) = polygon(&[[0.0, 0.0], [20.0, 0.0], [0.0, 20.0]]);
    let created = client.call(
        "apply",
        json!({"program":{"operations":[{
            "operation":"create_sketch","definition_id":definition,"name":"Wedge profile",
            "workplane":frame([123.6,-44.8,70.0]),"entities":entities,"constraints":constraints
        }]}}),
        true,
    );
    let sketch = created["created"]["feature_ids"][1].as_u64().unwrap();
    let created = client.call("apply", json!({"program":{"operations":[{
        "operation":"append_feature","definition_id":definition,"name":"Wedge pocket",
        "feature":{"type":"pocket","target_feature_id":pad,"profile_feature_id":sketch,"depth_mm":4.0}
    }]}}), true);
    let pocket = created["created"]["feature_ids"][0].as_u64().unwrap();
    let digest = client.state["canonical_digest"].clone();
    let result = client.call("evaluate", json!({"timeout_ms":30000}), false);
    assert!((native_volume(&result, pocket) - 3200.0).abs() < 1e-6);
    client.call("undo", json!({}), true);
    assert_ne!(client.state["canonical_digest"], digest);
    client.call("redo", json!({}), true);
    assert_eq!(client.state["canonical_digest"], digest);
    let path = std::env::temp_dir().join(format!(
        "ketchup-free-workplane-{}.ketchup",
        std::process::id()
    ));
    client.call("save", json!({"path":path,"overwrite":true}), true);
    client.call("open", json!({"path":path}), true);
    assert_eq!(client.state["canonical_digest"], digest);
    let result = client.call("evaluate", json!({"timeout_ms":30000}), false);
    assert!((native_volume(&result, pocket) - 3200.0).abs() < 1e-6);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn native_headless_topology_query_detail_drives_one_multi_edge_fillet() {
    let mut client = Client::new();
    let created = client.call(
        "apply",
        json!({"program":{"operations":[{
            "operation":"create_part","name":"Annular part",
            "workplane":{"type":"principal","plane":"xy"},
            "entities":[
                {"type":"circle","id":1,"center_mm":[12,-7],"radius_mm":10},
                {"type":"circle","id":2,"center_mm":[12,-7],"radius_mm":6}
            ],
            "constraints":[
                {"type":"radius","id":1,"entity_id":1,"value_mm":10},
                {"type":"radius","id":2,"entity_id":2,"value_mm":6}
            ],
            "feature":{"type":"extrusion","distance_mm":30},
            "translation_mm":[0,0,0]
        }]}}),
        true,
    );
    let definition = created["created"]["definition_ids"][0].as_u64().unwrap();
    let pad = created["created"]["feature_ids"][2].as_u64().unwrap();
    let evaluated = client.call("evaluate", json!({"timeout_ms":30000}), false);
    assert_eq!(evaluated["complete"], true, "{evaluated}");
    assert_eq!(evaluated["topology_complete"], true, "{evaluated}");

    let page = client.call(
        "query",
        json!({"kind":"edges","limit":100,"search":"circle","definition_id":definition}),
        false,
    );
    let mut upper_edges = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| {
            edge["producer_feature_id"] == pad
                && edge["geometry"]["closed"] == true
                && edge["geometry"]["centroid_mm"][2]
                    .as_f64()
                    .is_some_and(|z| (z - 30.0).abs() <= 1.0e-9)
        })
        .collect::<Vec<_>>();
    upper_edges.sort_by(|left, right| {
        left["geometry"]["circle_radius_mm"]
            .as_f64()
            .unwrap()
            .total_cmp(&right["geometry"]["circle_radius_mm"].as_f64().unwrap())
    });
    assert_eq!(upper_edges.len(), 2);
    assert_eq!(upper_edges[0]["geometry"]["circle_radius_mm"], 6.0);
    assert_eq!(upper_edges[1]["geometry"]["circle_radius_mm"], 10.0);
    let edge_id = upper_edges[0]["id"].as_u64().unwrap();
    assert!(edge_id > 0);
    let detail = client.call("detail", json!({"kind":"edges","id":edge_id}), false);
    assert_eq!(detail["item"], *upper_edges[0]);
    assert_eq!(detail["identity"], page["identity"]);

    let reference_ids = upper_edges
        .iter()
        .map(|edge| edge["reference_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    client.call(
        "apply",
        json!({"program":{"operations":[{
            "operation":"fillet_edges","definition_id":definition,"name":"Upper rim fillet",
            "target_feature_id":pad,"edge_reference_ids":reference_ids,"radius_mm":1
        }]},"selection":[]}),
        true,
    );
    let evaluated = client.call("evaluate", json!({"timeout_ms":30000}), false);
    assert_eq!(evaluated["complete"], true, "{evaluated}");
    assert_eq!(evaluated["topology_complete"], true, "{evaluated}");
}
