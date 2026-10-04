//! The server against a stand-in window: a real attach endpoint and live
//! bridge on loopback that answer like Kečup and record what they received.
#[path = "measurement_tests.rs"]
mod measurement;
use crate::{server::handle, tools::Tools};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    sync::mpsc,
};

#[test]
fn list_validators_is_discoverable_read_only_and_routes_to_the_host() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let listed = handle(
        &mut tools,
        json!({"jsonrpc":"2.0", "id":1, "method":"tools/list"}),
    )
    .unwrap();
    let tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "list_validators")
        .unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    assert!(crate::schema::INSTRUCTIONS.contains("list_validators"));
    assert_eq!(
        call(&mut tools, "list_validators", json!({}))["isError"],
        false
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method":"list_validators"})
    );
    assert_eq!(
        call(
            &mut tools,
            "list_validators",
            json!({"validators":["collision"]})
        )["isError"],
        true
    );
    assert!(
        received.try_recv().is_err(),
        "invalid catalog arguments never reach the host"
    );
}

#[test]
fn compact_program_schema_docs_and_routes_agree() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let schemas = crate::schema::tools();
    let schema = schemas
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "program")
        .unwrap();
    let properties = &schema["inputSchema"]["properties"];
    for action in ["read", "apply", "patch", "report", "docs", "validate"] {
        assert!(
            properties["action"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == action)
        );
    }
    for mode in properties["mode"]["enum"].as_array().unwrap() {
        let result = call(&mut tools, "program", json!({"action":"read", "mode":mode}));
        assert_eq!(result["isError"], false);
        let request = received.recv().unwrap();
        if mode == "full" {
            assert_eq!(request["method"], "program");
        } else {
            assert_eq!(request["method"], "program_context");
            assert_eq!(request["selection_context"], mode == "selection");
        }
    }
    let stamp = json!({"document_id":1,"revision":7,"canonical_digest":"d","mutation_epoch":0});
    let edits = json!([{"old":"width=600","new":"width=650"}]);
    call(
        &mut tools,
        "program",
        json!({"action":"patch","expected":stamp,"edits":edits}),
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method":"patch_program","expected":stamp,"edits":edits})
    );
    for section in properties["section"]["enum"].as_array().unwrap() {
        call(
            &mut tools,
            "program",
            json!({"action":"report","expected":stamp,"section":section}),
        );
        assert_eq!(
            received.recv().unwrap(),
            json!({"method":"program_report","expected":stamp,"section":section,"limit":50})
        );
    }
    let concise = text(&call(
        &mut tools,
        "program",
        json!({"action":"docs","name":"basics"}),
    ));
    let full = text(&call(
        &mut tools,
        "program",
        json!({"action":"docs","name":"basics","detail":"implementation"}),
    ));
    assert_eq!(concise["detail"], "concise");
    assert!(concise["text"].as_str().unwrap().len() < full["text"].as_str().unwrap().len());
    let (_, report) = ketchup_program::run(
        "docs.star",
        concise["example"].as_str().unwrap(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(report.bom.total_parts, 2);
    assert!(!report.bom.hardware.is_empty());
    assert_eq!(report.errors, 0);
}

#[test]
fn bounded_program_read_rejects_oversize_utf8_errors_and_special_files() {
    use crate::{
        bridge::MAX_REQUEST_BYTES,
        tools::{read_bounded_source, read_program_source},
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.star");
    std::fs::write(&path, "# žltý\\\"\n").unwrap();
    assert_eq!(read_program_source(&path).unwrap(), "# žltý\\\"\n");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(1024 * 1024 * 1024)
        .unwrap();
    assert!(read_program_source(&path).is_err());
    assert!(read_program_source(root.path()).is_err());
    assert!(read_program_source(Path::new("relative.star")).is_err());
    assert!(read_bounded_source(&[0xff][..]).is_err());
    let mut endless = std::io::repeat(b'x');
    let mut counted = (&mut endless).take((MAX_REQUEST_BYTES * 10) as u64);
    assert!(read_bounded_source(&mut counted).is_err());
    assert_eq!(
        counted.limit(),
        (MAX_REQUEST_BYTES * 9 - 1) as u64,
        "consume only limit+1 bytes"
    );
    #[cfg(windows)]
    assert!(read_program_source(Path::new(r"\\.\NUL")).is_err());
    #[cfg(unix)]
    {
        assert!(read_program_source(Path::new("/dev/zero")).is_err());
        let socket = root.path().join("socket");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert!(read_program_source(&socket).is_err());
        let link = root.path().join("link");
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        assert!(read_program_source(&link).is_err());
    }
}

#[test]
fn escaped_source_envelope_is_refused_before_send_and_connection_remains_usable() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let path = root.path().join("escaped.star");
    std::fs::write(&path, "\u{1}".repeat(crate::bridge::MAX_REQUEST_BYTES / 2)).unwrap();
    let result = call(
        &mut tools,
        "program",
        json!({"action":"apply", "source_path":path}),
    );
    assert_eq!(text(&result)["error"], "request_too_large");
    let source = "# žltý \\\"\n";
    std::fs::write(&path, source).unwrap();
    assert_eq!(
        call(
            &mut tools,
            "program",
            json!({"action":"apply", "source_path":path})
        )["isError"],
        false
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method":"apply_program", "source":source})
    );
}

#[test]
fn concurrent_launch_readiness_is_bound_to_each_private_stream_not_registration_order() {
    let first = TcpListener::bind("127.0.0.1:0").unwrap();
    let second = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut first_writer = std::net::TcpStream::connect(first.local_addr().unwrap()).unwrap();
    let mut second_writer = std::net::TcpStream::connect(second.local_addr().unwrap()).unwrap();
    let first_reader = first.accept().unwrap().0;
    let second_reader = second.accept().unwrap().0;
    let a = "a".repeat(32);
    let b = "b".repeat(32);
    let ready = |id: &str, port| {
        format!(
            "{}\n",
            json!({"version":1,"instance_id":id,"consent_address":"127.0.0.1:1234","live_bridge_address":format!("127.0.0.1:{port}")})
        )
    };
    let waiting = std::thread::spawn(move || {
        crate::discovery::launched_window(first_reader, "A".into()).unwrap()
    });
    second_writer.write_all(ready(&b, 2222).as_bytes()).unwrap();
    let (second_window, second_address) =
        crate::discovery::launched_window(second_reader, "B".into()).unwrap();
    first_writer.write_all(ready(&a, 1111).as_bytes()).unwrap();
    let (first_window, first_address) = waiting.join().unwrap();
    assert_eq!(
        (
            first_window.instance_id,
            first_window.document,
            first_address.port()
        ),
        (a, "A".into(), 1111)
    );
    assert_eq!(
        (
            second_window.instance_id,
            second_window.document,
            second_address.port()
        ),
        (b, "B".into(), 2222)
    );
    assert!(crate::discovery::launched_window(&b"{}\n"[..], "missing".into()).is_err());
    assert!(crate::discovery::launched_window(&vec![b'x'; 513][..], "oversize".into()).is_err());
}

const INSTANCE: &str = "0123456789abcdef0123456789abcdef";
const TOKEN: &str = "abababababababababababababababababababababababababababababababab";

/// Publishes a window in `root`; returns the requests its bridge receives.
fn stand_in_window(root: &Path) -> mpsc::Receiver<Value> {
    let attach = TcpListener::bind("127.0.0.1:0").unwrap();
    let bridge = TcpListener::bind("127.0.0.1:0").unwrap();
    let bridge_address = bridge.local_addr().unwrap().to_string();
    let mut registry = crate::local_auth::create(&root.join(format!("{INSTANCE}.json"))).unwrap();
    write!(
        registry,
        "{}",
        json!({"version": 1, "instance_id": INSTANCE,
        "consent_address": attach.local_addr().unwrap().to_string(), "bootstrap": TOKEN})
    )
    .unwrap();
    drop(registry);
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
                assert_eq!(request["bootstrap"], TOKEN);
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
            "list_validators",
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
