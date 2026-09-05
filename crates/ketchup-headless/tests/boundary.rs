use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn exchange(lines: &[String]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ketchup-headless"))
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for line in lines {
            writeln!(stdin, "{line}").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn request(id: u64, method: &str, params: Value) -> String {
    json!({"protocol":"ketchup.headless.v1","id":id,"method":method,"params":params}).to_string()
}

#[test]
fn live_protocol_rejects_duplicate_permissions_and_stays_synchronized() {
    let responses = exchange(&[
        request(1, "state", json!({})),
        r#"{"protocol":"ketchup.headless.v1","id":2,"method":"save","params":{"overwrite":false,"overwrite":true}}"#.to_owned(),
        r#"{"protocol":"ketchup.headless.v1","id":3,"method":"new","params":{"discard_unsaved":false,"discard_unsaved":true}}"#.to_owned(),
        r#"{"protocol":"ketchup.headless.v1","id":4,"method":"state","method":"new","params":{}}"#.to_owned(),
        request(5, "state", json!({})),
    ]);
    assert_eq!(responses.len(), 5);
    for response in &responses[1..4] {
        assert_eq!(response["error"]["code"], "invalid_json");
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("duplicate JSON field")
        );
    }
    assert_eq!(
        responses[0]["result"]["state"],
        responses[4]["result"]["state"]
    );
    assert_eq!(responses[4]["id"], 5);
}

#[test]
fn live_protocol_honors_nondefault_deadline_and_rejects_out_of_range() {
    let responses = exchange(&[
        request(1, "state", json!({})),
        request(2, "evaluate", json!({"timeout_ms":2500})),
        request(3, "evaluate", json!({"timeout_ms":0})),
        request(4, "evaluate", json!({"timeout_ms":300001})),
        request(5, "state", json!({})),
    ]);
    assert_eq!(responses.len(), 5);
    assert!(responses[1].get("error").is_none(), "{:?}", responses[1]);
    assert_eq!(responses[1]["result"]["complete"], false);
    assert!(responses[1]["result"]["not_evaluated"].is_string());
    for response in &responses[2..4] {
        assert_eq!(response["error"]["code"], "invalid_params");
    }
    assert_eq!(
        responses[0]["result"]["state"],
        responses[4]["result"]["state"]
    );
}

#[test]
fn advertised_color_schema_is_nullable_bounded_rgb() {
    let responses = exchange(&[request(1, "capabilities", json!({}))]);
    fn locate(value: &Value) -> Option<&Value> {
        if value.pointer("/properties/operation/const") == Some(&json!("set_color")) {
            return Some(value);
        }
        match value {
            Value::Object(map) => map.values().find_map(locate),
            Value::Array(items) => items.iter().find_map(locate),
            _ => None,
        }
    }
    let operation = locate(&responses[0]).expect("advertised set_color operation");
    let color = &operation["properties"]["color"]["anyOf"];
    assert_eq!(color[0]["minItems"], 3);
    assert_eq!(color[0]["maxItems"], 3);
    assert_eq!(
        color[0]["items"],
        json!({"type":"integer","minimum":0,"maximum":255})
    );
    assert_eq!(color[1], json!({"type":"null"}));
}
