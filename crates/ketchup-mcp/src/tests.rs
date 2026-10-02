//! The server against a stand-in window: a real attach endpoint and live
//! bridge on loopback that answer like Kečup and record what they received.
use crate::{server::handle, tools::Tools};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    sync::mpsc,
};

const INSTANCE: &str = "0123456789abcdef0123456789abcdef";
const TOKEN: &str = "abababababababababababababababababababababababababababababababab";

/// Publishes a window in `root`; returns the requests its bridge receives.
fn stand_in_window(root: &Path) -> mpsc::Receiver<Value> {
    let attach = TcpListener::bind("127.0.0.1:0").unwrap();
    let bridge = TcpListener::bind("127.0.0.1:0").unwrap();
    let bridge_address = bridge.local_addr().unwrap().to_string();
    std::fs::write(
        root.join(format!("{INSTANCE}.json")),
        json!({"version": 1, "instance_id": INSTANCE,
               "consent_address": attach.local_addr().unwrap().to_string()})
        .to_string(),
    )
    .unwrap();
    std::thread::spawn(move || {
        for stream in attach.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let mut reply = json!({"version": 1, "nonce": request["nonce"],
                                   "instance_id": INSTANCE});
            if request["action"] == "list" {
                reply["status"] = "available".into();
                reply["document"] = "stolik.ketchup".into();
            } else {
                reply["status"] = "allowed".into();
                reply["live_bridge_address"] = bridge_address.clone().into();
                reply["token"] = TOKEN.into();
            }
            stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
        }
    });
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = bridge.accept().unwrap();
        loop {
            let mut header = [0_u8; 4];
            if stream.read_exact(&mut header).is_err() {
                return;
            }
            let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
            stream.read_exact(&mut body).unwrap();
            let envelope: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(envelope["token"], TOKEN);
            let request = envelope["request"].clone();
            let stamp = json!({"document_id": 1, "revision": 7,
                               "canonical_digest": "d", "mutation_epoch": 0});
            let (ok, result, error) = match request["method"].as_str().unwrap() {
                "undo" => (
                    false,
                    json!({"code": "undo_unavailable", "phase": "history",
                    "target": "document", "reason": "There is nothing to undo.",
                    "fix_hint": "Make a change first.", "causes": []}),
                    json!("undo_unavailable"),
                ),
                "image" => (
                    true,
                    json!({"data": "iVBORw0KGgo=", "width": 512, "height": 384,
                    "framing": {"mode": "viewport"}, "view": {}, "selection": []}),
                    Value::Null,
                ),
                _ => (true, json!({"document": "stolik.ketchup"}), Value::Null),
            };
            sender.send(request).unwrap();
            let reply = json!({"version": 1, "id": envelope["id"], "ok": ok,
                               "stamp": stamp, "result": result, "error": error})
            .to_string();
            stream
                .write_all(&(reply.len() as u32).to_be_bytes())
                .unwrap();
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    received
}

fn call(tools: &mut Tools, name: &str, arguments: Value) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                         "params": {"name": name, "arguments": arguments}});
    handle(tools, request).unwrap()["result"].clone()
}

fn text(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn a_client_initializes_and_lists_every_tool() {
    let mut tools = Tools::new(None, None);
    let initialized = handle(
        &mut tools,
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
               "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
    )
    .unwrap();
    assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(initialized["result"]["serverInfo"]["name"], "ketchup");
    let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert!(handle(&mut tools, notification).is_none());
    let listed = handle(
        &mut tools,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .unwrap();
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "windows",
            "connect",
            "open_window",
            "program",
            "inspect",
            "model",
            "edit",
            "file",
            "view",
            "batch"
        ]
    );
    let unknown = handle(
        &mut tools,
        json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}),
    )
    .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);
}

#[test]
fn without_an_open_window_the_agent_is_told_what_to_do() {
    let root = tempfile::tempdir().unwrap();
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let result = call(&mut tools, "inspect", json!({"action": "status"}));
    assert_eq!(result["isError"], true);
    let error = text(&result);
    assert_eq!(error["error"], "no_window");
    assert!(error["message"].as_str().unwrap().contains("open_window"));
    let docs = call(
        &mut tools,
        "program",
        json!({"action": "docs", "name": "basics"}),
    );
    assert_eq!(docs["isError"], false, "docs need no window");
}

#[test]
fn tool_calls_reach_the_open_window_as_bridge_requests() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));

    let windows = text(&call(&mut tools, "windows", json!({})));
    assert_eq!(windows["windows"][0]["instance_id"], INSTANCE);
    assert_eq!(windows["windows"][0]["document"], "stolik.ketchup");

    // The first call attaches by itself because exactly one window is open.
    let status = call(
        &mut tools,
        "inspect",
        json!({"action": "status", "expected": null}),
    );
    assert_eq!(status["isError"], false);
    assert_eq!(text(&status)["result"]["document"], "stolik.ketchup");
    assert_eq!(text(&status)["stamp"]["revision"], 7);
    assert_eq!(received.recv().unwrap(), json!({"method": "status"}));

    let source = "board(\"top\", (600, 400, 18))\n";
    call(
        &mut tools,
        "program",
        json!({"action": "apply", "source": source,
                                       "overrides": {"width": 900}}),
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method": "apply_program", "source": source, "overrides": {"width": 900}})
    );

    call(
        &mut tools,
        "inspect",
        json!({"action": "query", "kind": "occurrences", "limit": 5}),
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method": "query", "query": {"kind": "occurrences", "limit": 5}})
    );

    let undo = call(&mut tools, "edit", json!({"action": "undo"}));
    assert_eq!(undo["isError"], true);
    let refused = text(&undo);
    assert_eq!(refused["error"], "undo_unavailable");
    assert_eq!(refused["fix_hint"], "Make a change first.");
    received.recv().unwrap();

    let image = call(&mut tools, "view", json!({"action": "image"}));
    assert_eq!(image["content"][0]["type"], "image");
    assert_eq!(image["content"][0]["data"], "iVBORw0KGgo=");
    assert_eq!(image["content"][0]["mimeType"], "image/png");
    assert_eq!(
        text(&json!({"content": [image["content"][1]]}))["width"],
        512
    );
    let image_request = received.recv().unwrap();
    assert_eq!(image_request["framing"], "viewport");
    assert_eq!(image_request["capture_mode"], "offscreen");

    // Schema defaults a client fills in must not turn selection framing
    // into a detail request.
    call(
        &mut tools,
        "view",
        json!({"action": "image", "framing": "selection", "detail_occurrence_id": 0,
               "detail_kind": "", "detail_entity_id": 0, "view": ""}),
    );
    let image_request = received.recv().unwrap();
    assert_eq!(image_request["framing"], "selection");
    assert!(
        image_request.get("detail_target").is_none(),
        "{image_request}"
    );
    assert!(image_request.get("view").is_none(), "{image_request}");
}
