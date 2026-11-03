//! The server against a stand-in window: a real attach endpoint and live
//! bridge on loopback that answer like Kečup and record what they received.
#[path = "measurement_tests.rs"]
mod measurement;
use crate::{
    server::{Session, handle},
    tools::Tools,
};
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
        &Session::new(&mut tools),
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
fn a_section_through_the_origin_keeps_its_zero_offset() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    call(
        &mut tools,
        "view",
        json!({"action":"section","normal":[0,0,1],"offset_mm":0}),
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method":"section","normal":[0,0,1],"offset_mm":0})
    );
    call(
        &mut tools,
        "view",
        json!({"action":"close_section","normal":[0,0,1],"offset_mm":0}),
    );
    assert_eq!(received.recv().unwrap(), json!({"method":"section"}));
}

#[test]
fn run_plugin_is_listed_and_reaches_the_window_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let schemas = crate::schema::tools();
    assert!(
        schemas
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "run_plugin")
    );
    let arguments = json!({"executable": "C:\\Python\\python.exe",
        "arguments": ["C:\\plugins\\height.py", "2", "35"], "principal_id": 7001,
        "capabilities": ["query.agent-state.v1", "edit.cad-program.v1"],
        "allowed_operations": ["set_dimension"], "timeout_ms": 30000});
    let folder = json!({"plugin": "C:\\plugins\\height", "inputs": {"height_mm": 35}});
    for arguments in [arguments, folder] {
        call(&mut tools, "run_plugin", arguments.clone());
        let mut sent = arguments;
        sent["method"] = "run_plugin".into();
        assert_eq!(received.recv().unwrap(), sent);
    }
}

/// Review 2026-10-17 P3: docs/plugins.md described only plugin folders, and the
/// instructions' import summary left out DWG, which `file import` reads.
#[test]
fn plugins_doc_and_instructions_name_what_the_tools_take() {
    let schemas = crate::schema::tools();
    let tool = |name: &str| {
        schemas
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap()
            .clone()
    };
    let doc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/plugins.md"),
    )
    .unwrap();
    let run_plugin = tool("run_plugin");
    for field in run_plugin["inputSchema"]["properties"]
        .as_object()
        .unwrap()
        .keys()
    {
        assert!(
            doc.contains(&format!("`{field}`")),
            "docs/plugins.md does not describe run_plugin {field}"
        );
    }
    let file = tool("file");
    let imports = file["description"].as_str().unwrap();
    let (_, after) = crate::schema::INSTRUCTIONS
        .split_once("file action=import")
        .unwrap();
    let summary = after.split(';').next().unwrap();
    for format in ["STEP", "IGES", "STL", "DXF", "DWG", "GLB", "SketchUp"] {
        assert!(imports.contains(format), "{format} not in {imports}");
        assert!(summary.contains(format), "{format} not in {summary}");
    }
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
        } else if mode == "outline" {
            assert_eq!(request["method"], "program_piece");
            assert_eq!(request["outline"], true);
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
    {
        assert!(read_program_source(Path::new(r"\\.\NUL")).is_err());
        // Only a drive path is read: the same readable file named through a
        // share or device prefix is refused before the file system sees it.
        let small = root.path().join("small.star");
        std::fs::write(&small, "box('a', (1, 1, 1))\n").unwrap();
        assert!(read_program_source(&small).is_ok());
        let verbatim = format!(r"\\?\{}", small.display());
        let error = read_program_source(Path::new(&verbatim)).unwrap_err();
        assert!(
            format!("{error:?}").contains("local drive"),
            "{verbatim}: {error:?}"
        );
    }
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

/// Review 2026-10-09 §5.1 (3): check sends a whole program, a file or a patch
/// to the window's dry run, never to apply.
#[test]
fn program_check_sends_source_file_or_patch_to_the_dry_run() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let path = root.path().join("check.star");
    let source = "box(\"a\", (10, 10, 10))\n";
    std::fs::write(&path, source).unwrap();
    let stamp = json!({"document_id":1,"revision":7,"canonical_digest":"d","mutation_epoch":0});
    let edits = json!([{"old":"10, 10, 10","new":"10, 10, 20"}]);
    for (arguments, sent) in [
        (
            json!({"action":"check", "source":source}),
            json!({"method":"check_program", "source":source}),
        ),
        (
            json!({"action":"check", "source_path":path}),
            json!({"method":"check_program", "source":source}),
        ),
        (
            json!({"action":"check", "expected":stamp, "edits":edits}),
            json!({"method":"check_program", "expected":stamp, "edits":edits}),
        ),
    ] {
        let result = call(&mut tools, "program", arguments);
        assert_eq!(result["isError"], false, "{result}");
        let request = received.recv().unwrap();
        if request["method"] == "status" {
            assert_eq!(received.recv().unwrap(), sent);
        } else {
            assert_eq!(request, sent);
        }
    }
}

/// Review 2026-10-09 §5.1 (5): pieces of a large program and new parameter
/// values travel without the whole source.
#[test]
fn program_pieces_and_set_params_reach_their_window_methods() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let stamp = json!({"document_id":1,"revision":7,"canonical_digest":"d","mutation_epoch":0});
    for (arguments, sent) in [
        (
            json!({"action":"read", "lines":[10, 20]}),
            json!({"method":"program_piece", "lines":[10, 20]}),
        ),
        (
            json!({"action":"read", "search":"leg"}),
            json!({"method":"program_piece", "search":"leg"}),
        ),
        (
            json!({"action":"read", "part":"seat"}),
            json!({"method":"program_piece", "part":"seat"}),
        ),
        (
            json!({"action":"read", "mode":"outline"}),
            json!({"method":"program_piece", "outline":true}),
        ),
        (
            json!({"action":"set_params", "expected":stamp, "params":{"width":900}}),
            json!({"method":"set_program_params", "expected":stamp, "params":{"width":900}}),
        ),
        (
            json!({"action":"save_variant", "expected":stamp, "name":"800"}),
            json!({"method":"save_program_variant", "expected":stamp, "name":"800"}),
        ),
        (
            json!({"action":"use_variant", "expected":stamp, "name":"800"}),
            json!({"method":"use_program_variant", "expected":stamp, "name":"800"}),
        ),
        (
            json!({"action":"delete_variant", "expected":stamp, "name":"800"}),
            json!({"method":"delete_program_variant", "expected":stamp, "name":"800"}),
        ),
        (
            json!({"action":"export_variants", "expected":stamp, "path":"C:\\out\\cabinet.csv"}),
            json!({"method":"export_program_variants", "expected":stamp, "path":"C:\\out\\cabinet.csv"}),
        ),
    ] {
        let result = call(&mut tools, "program", arguments);
        assert_eq!(result["isError"], false, "{result}");
        let request = received.recv().unwrap();
        if request["method"] == "status" {
            assert_eq!(received.recv().unwrap(), sent);
        } else {
            assert_eq!(request, sent);
        }
    }
    let mixed = call(
        &mut tools,
        "program",
        json!({"action":"read", "mode":"selection", "lines":[1, 2]}),
    );
    assert_eq!(mixed["isError"], true, "{mixed}");
}

#[test]
fn a_large_first_program_is_applied_after_a_small_status_authenticates_the_connection() {
    let root = tempfile::tempdir().unwrap();
    let received = stand_in_window(root.path());
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let source = format!(
        "box(\"a\", (10, 10, 10))\n{}\n",
        "#".repeat(crate::MAX_UNAUTHENTICATED_REQUEST_BYTES * 2)
    );
    for _ in 0..2 {
        let result = call(
            &mut tools,
            "program",
            json!({"action":"apply", "source":source}),
        );
        assert_eq!(result["isError"], false, "{result}");
    }
    assert_eq!(received.recv().unwrap(), json!({"method":"status"}));
    for _ in 0..2 {
        assert_eq!(
            received.recv().unwrap(),
            json!({"method":"apply_program", "source":source})
        );
    }
    assert!(
        received.try_recv().is_err(),
        "an authenticated connection sends no further status"
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
    let failed = br#"{"version":1,"error":"live bridge bootstrap failed: the requested document is open in another Kecup window"}
"#;
    let error = crate::discovery::launched_window(&failed[..], "in use".into()).unwrap_err();
    assert!(
        error.to_string().contains("open in another Kecup window"),
        "{error}"
    );
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
                let nonce = request["nonce"].as_str().unwrap();
                assert_eq!(
                    request["proof"],
                    crate::local_auth::client_list_proof(TOKEN, INSTANCE, nonce)
                );
                reply["status"] = "available".into();
                reply["document"] = "stolik.ketchup".into();
                reply["proof"] = crate::discovery::window_list_proof_of(TOKEN, &reply).into();
            } else {
                let nonce = request["nonce"].as_str().unwrap();
                assert_eq!(
                    request["proof"],
                    crate::local_auth::client_attach_proof(TOKEN, INSTANCE, nonce)
                );
                assert!(request.get("bootstrap").is_none());
                reply["status"] = "allowed".into();
                reply["live_bridge_address"] = bridge_address.clone().into();
                reply["token"] = TOKEN.into();
                reply["proof"] = crate::discovery::window_proof(TOKEN, &reply).into();
            }
            stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
        }
    });
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = bridge.accept().unwrap();
        let mut authenticated = false;
        loop {
            let mut header = [0_u8; 4];
            if stream.read_exact(&mut header).is_err() {
                return;
            }
            // Like the window: a large frame before authentication ends the connection.
            if !authenticated
                && u32::from_be_bytes(header) as usize > crate::MAX_UNAUTHENTICATED_REQUEST_BYTES
            {
                return;
            }
            authenticated = true;
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
                // `max_side_px: 1` stands for a reply that lost its picture.
                "image" if request["max_side_px"] == 1 => (
                    true,
                    json!({"width": 1, "height": 1, "framing": {"mode": "viewport"},
                    "view": {}, "selection": []}),
                    Value::Null,
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

/// SEC-6: after a window closed, another program can take its attach address.
/// It learns no bootstrap from the attach and cannot pass for the window,
/// however well it answers.
#[test]
fn a_program_holding_a_closed_windows_address_cannot_pass_for_it() {
    let root = tempfile::tempdir().unwrap();
    let impostor = TcpListener::bind("127.0.0.1:0").unwrap();
    let impostor_address = impostor.local_addr().unwrap();
    let mut registry =
        crate::local_auth::create(&root.path().join(format!("{INSTANCE}.json"))).unwrap();
    write!(
        registry,
        "{}",
        json!({"version": 1, "instance_id": INSTANCE,
        "consent_address": impostor.local_addr().unwrap().to_string(), "bootstrap": TOKEN})
    )
    .unwrap();
    drop(registry);
    let (seen, requests) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in impostor.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let reply = json!({"version": 1, "nonce": request["nonce"], "instance_id": INSTANCE,
                               "status": "allowed", "document": "stolik.ketchup",
                               "live_bridge_address": "127.0.0.1:9", "token": "cd".repeat(32),
                               "proof": request["proof"]});
            seen.send(line).unwrap();
            stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
        }
    });

    // Review 2026-10-19 P3-C: the impostor's list answer has no window proof,
    // so it names no document and no window is listed.
    assert!(crate::discovery::list_windows(root.path()).is_empty());
    let window = crate::discovery::Window {
        registry_path: root.path().join(format!("{INSTANCE}.json")),
        instance_id: INSTANCE.to_owned(),
        document: String::new(),
        in_use: false,
        attach_address: impostor_address,
    };
    let error = crate::discovery::attach(&window)
        .err()
        .expect("the impostor's grant is refused");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    let attach_request = requests.iter().nth(1).unwrap();
    assert!(!attach_request.contains(TOKEN), "{attach_request}");
}

/// How a scripted window ends one connection after answering its first request.
enum Ending {
    /// Answers the second request: another client took the window over meanwhile.
    SupersededInFlight,
    /// The same from a window that says the document changed while it ran.
    SupersededAfterAChange,
    /// Tells the idle client it was taken over, before its next request.
    SupersededIdle,
    /// Just closes, like a window that ends the connection for another reason.
    Close,
    /// Never answers the second request; the connection ends when the client leaves.
    Silent,
}

/// A window `instance` that ends its successive connections as `endings` say.
/// Returns one message per attach and one per ended connection.
fn scripted_window(
    root: &Path,
    instance: &'static str,
    endings: Vec<Ending>,
) -> mpsc::Receiver<&'static str> {
    let bridge = TcpListener::bind("127.0.0.1:0").unwrap();
    let (events, received) = mpsc::channel();
    grant_attaches(root, instance, &bridge, events.clone());
    std::thread::spawn(move || {
        let frame = |stream: &mut std::net::TcpStream, value: Value| {
            let body = value.to_string();
            stream
                .write_all(&(body.len() as u32).to_be_bytes())
                .unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        };
        let read_id = |stream: &mut std::net::TcpStream| read_frame(stream)["id"].clone();
        let stamp = json!({"document_id": 1, "revision": 7, "canonical_digest": "d",
                           "mutation_epoch": 0});
        for ending in endings {
            let (mut stream, _) = bridge.accept().unwrap();
            let id = read_id(&mut stream);
            frame(
                &mut stream,
                json!({"version": 1, "id": id, "ok": true, "stamp": stamp, "result": {}}),
            );
            let superseded = |id: Value| {
                json!({"version": 1, "id": id, "ok": false,
                       "error": crate::SUPERSEDED_BY_ANOTHER_CLIENT})
            };
            match ending {
                Ending::SupersededInFlight => {
                    let id = read_id(&mut stream);
                    frame(&mut stream, superseded(id));
                }
                Ending::SupersededAfterAChange => {
                    let id = read_id(&mut stream);
                    let mut last = superseded(id);
                    last["stamp"] = json!({"revision": 9});
                    last["result"] = json!({"document_changed": true});
                    frame(&mut stream, last);
                }
                Ending::SupersededIdle => frame(&mut stream, superseded(json!(0))),
                Ending::Close => {}
                Ending::Silent => {
                    read_id(&mut stream);
                    events.send("waiting").expect("the test listens");
                    let after = stream.read_to_end(&mut Vec::new()).map_or(0, |bytes| bytes);
                    assert_eq!(after, 0, "the client sends nothing more");
                }
            }
            drop(stream);
            let _ = events.send("ended");
        }
    });
    received
}

/// One framed request body the client sent.
fn read_frame(stream: &mut std::net::TcpStream) -> Value {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).unwrap();
    let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// Registers window `instance` under `root` and grants every attach to it on
/// `bridge`, sending "attach" to `attached` for each.
fn grant_attaches(
    root: &Path,
    instance: &'static str,
    bridge: &TcpListener,
    attached: mpsc::Sender<&'static str>,
) {
    let attach = TcpListener::bind("127.0.0.1:0").unwrap();
    let bridge_address = bridge.local_addr().unwrap().to_string();
    let mut registry = crate::local_auth::create(&root.join(format!("{instance}.json"))).unwrap();
    write!(
        registry,
        "{}",
        json!({"version": 1, "instance_id": instance,
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
            let mut reply = json!({"version": 1, "nonce": request["nonce"], "instance_id": instance,
                                   "status": "available", "document": "stolik.ketchup"});
            if request["action"] == "attach" {
                reply["status"] = "allowed".into();
                reply["live_bridge_address"] = bridge_address.clone().into();
                reply["token"] = TOKEN.into();
                reply["proof"] = crate::discovery::window_proof(TOKEN, &reply).into();
                let _ = attached.send("attach");
            } else {
                reply["proof"] = crate::discovery::window_list_proof_of(TOKEN, &reply).into();
            }
            stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
        }
    });
}

/// #2687 P1 (point 4): the window is told which AI attached, in the words the
/// client's users know, before the call that attached; a window too old to
/// know `hello` refuses it and the call still goes through.
#[test]
fn the_window_hears_which_ai_attached_before_its_first_call() {
    let root = tempfile::tempdir().unwrap();
    let bridge = TcpListener::bind("127.0.0.1:0").unwrap();
    grant_attaches(root.path(), INSTANCE, &bridge, mpsc::channel().0);
    let (requests, received) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = bridge.accept().unwrap();
        for _ in 0..2 {
            let envelope = read_frame(&mut stream);
            let request = envelope["request"].clone();
            let reply = if request["method"] == "hello" {
                json!({"version": 1, "id": envelope["id"], "ok": false, "error": "invalid_params",
                       "result": {"reason": "unknown variant `hello`"}})
            } else {
                json!({"version": 1, "id": envelope["id"], "ok": true, "result": {},
                       "stamp": {"document_id": 1, "revision": 7, "canonical_digest": "d",
                                 "mutation_epoch": 0}})
            };
            let body = reply.to_string();
            stream
                .write_all(&(body.len() as u32).to_be_bytes())
                .unwrap();
            stream.write_all(body.as_bytes()).unwrap();
            requests.send(request).unwrap();
        }
    });
    let session = Session::new(Tools::new(None, Some(root.path().to_owned())));
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                            "params": {"clientInfo": {"name": "claude-code", "version": "2.1"}}});
    assert!(handle(&session, initialize).is_some());
    let status = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                        "params": {"name": "inspect", "arguments": {"action": "status"}}});
    assert_eq!(
        handle(&session, status).unwrap()["result"]["isError"],
        false
    );
    assert_eq!(
        received.recv().unwrap(),
        json!({"method": "hello", "client": "Claude Code"})
    );
    assert_eq!(received.recv().unwrap()["method"], "status");
}

/// MCP-1: a window another client took over is not taken back behind the
/// user's back, whether the takeover came during a request or between two.
#[test]
fn a_taken_over_window_is_not_reconnected_silently() {
    let root = tempfile::tempdir().unwrap();
    let events = scripted_window(
        root.path(),
        INSTANCE,
        vec![Ending::SupersededInFlight, Ending::SupersededIdle],
    );
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let status = json!({"action": "status"});
    assert_eq!(
        call(&mut tools, "inspect", status.clone())["isError"],
        false
    );
    let in_flight = text(&call(&mut tools, "inspect", status.clone()));
    assert_eq!(in_flight["error"], "connection_taken_over");
    let message = in_flight["message"].as_str().unwrap();
    assert!(
        message.contains("may or may not have been applied"),
        "{message}"
    );
    assert_eq!(
        text(&call(&mut tools, "inspect", status.clone()))["error"],
        "connection_taken_over",
        "the next call does not take the window back"
    );
    assert_eq!(events.recv().unwrap(), "attach");
    assert_eq!(events.recv().unwrap(), "ended");
    assert!(events.try_recv().is_err(), "no second attach");

    assert_eq!(call(&mut tools, "connect", json!({}))["isError"], false);
    assert_eq!(events.recv().unwrap(), "attach");
    assert_eq!(events.recv().unwrap(), "ended");
    // The window wrote its last frame before it ended, but on a loaded machine
    // the frame can reach this side later (#152): the call must find it there.
    tools.wait_for_window_frame(std::time::Duration::from_secs(5));
    let idle = text(&call(&mut tools, "inspect", status));
    assert_eq!(idle["error"], "connection_taken_over");
    let message = idle["message"].as_str().unwrap();
    assert!(message.contains("was not sent"), "{message}");
    assert!(events.try_recv().is_err(), "no third attach");
}

/// MCP 16a: the window says whether the document changed while the request
/// that was running when another client took over ran, so the agent does not
/// have to guess; `windows` says the other client can be taken over (MCP 15).
#[test]
fn a_takeover_says_whether_the_request_changed_the_document() {
    let root = tempfile::tempdir().unwrap();
    let _events = scripted_window(root.path(), INSTANCE, vec![Ending::SupersededAfterAChange]);
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let status = json!({"action": "status"});
    assert_eq!(
        call(&mut tools, "inspect", status.clone())["isError"],
        false
    );
    let in_flight = text(&call(&mut tools, "inspect", status));
    assert_eq!(in_flight["error"], "connection_taken_over");
    let message = in_flight["message"].as_str().unwrap();
    assert!(message.contains("probably applied"), "{message}");
    assert_eq!(in_flight["details"]["stamp"]["revision"], 9, "{in_flight}");

    let held = json!({"instance_id": INSTANCE, "used_by_another_client": true});
    let listed = crate::tools::listed_windows(vec![held]);
    let note = listed["used_by_another_client"].as_str().unwrap();
    assert!(note.contains("connect takes such a window over"), "{note}");
    let free = json!({"instance_id": INSTANCE, "used_by_another_client": false});
    assert!(
        crate::tools::listed_windows(vec![free])
            .get("used_by_another_client")
            .is_none()
    );
}

/// Answers every request on `bridge` as done, like a window that has nothing to say.
fn answer_every_request(bridge: TcpListener) {
    std::thread::spawn(move || {
        for stream in bridge.incoming() {
            let mut stream = stream.unwrap();
            loop {
                let mut header = [0_u8; 4];
                if stream.read_exact(&mut header).is_err() {
                    break;
                }
                let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
                stream.read_exact(&mut body).unwrap();
                let envelope: Value = serde_json::from_slice(&body).unwrap();
                let reply = json!({"version": 1, "id": envelope["id"], "ok": true, "result": {},
                                   "stamp": {"document_id": 1, "revision": 7,
                                             "canonical_digest": "d", "mutation_epoch": 0}})
                .to_string();
                stream
                    .write_all(&(reply.len() as u32).to_be_bytes())
                    .unwrap();
                stream.write_all(reply.as_bytes()).unwrap();
            }
        }
    });
}

/// #2687 point 16: with several windows open, `connect` without an id goes to
/// the window whose AI tab Connect was clicked in. Without that click, or once
/// that window closed, it asks for a choice and names every window by the short
/// name the window's AI tab shows.
#[test]
fn connect_without_an_id_goes_to_the_window_connect_was_clicked_in() {
    const OTHER: &str = "fedcba9876543210fedcba9876543210";
    let root = tempfile::tempdir().unwrap();
    let window = |instance| {
        let bridge = TcpListener::bind("127.0.0.1:0").unwrap();
        let (events, received) = mpsc::channel();
        grant_attaches(root.path(), instance, &bridge, events);
        answer_every_request(bridge);
        received
    };
    let first = window(INSTANCE);
    let second = window(OTHER);
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let short = crate::discovery::short_name;

    let choose = text(&call(&mut tools, "connect", json!({})));
    assert_eq!(choose["error"], "choose_window", "{choose}");
    let names: Vec<Value> = choose["details"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| window["short_name"].clone())
        .collect();
    assert_eq!(names, [json!(short(INSTANCE)), json!(short(OTHER))]);

    crate::discovery::mark_chosen(root.path(), OTHER).unwrap();
    let connected = call(&mut tools, "connect", json!({}));
    assert_eq!(connected["isError"], false, "{connected}");
    assert_eq!(
        second
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
        "attach"
    );
    assert!(first.try_recv().is_err(), "the other window is left alone");
    let listed = text(&call(&mut tools, "windows", json!({})));
    let chosen: Vec<Value> = listed["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| window["chosen"].clone())
        .collect();
    assert_eq!(chosen, [json!(false), json!(true)], "{listed}");

    // The window Connect was clicked in closed; another one is not guessed.
    crate::discovery::mark_chosen(root.path(), &"a".repeat(32)).unwrap();
    let choose = text(&call(&mut tools, "connect", json!({})));
    assert_eq!(choose["error"], "choose_window", "{choose}");
    assert!(choose["details"].get("chosen_unreadable").is_none());

    // A damaged choice is named to the AI, not taken for "Connect clicked nowhere".
    let file = root.path().join("chosen-window.json");
    std::fs::remove_file(&file).unwrap();
    crate::local_auth::create(&file)
        .unwrap()
        .write_all(b"{\"version\": 1")
        .unwrap();
    let choose = text(&call(&mut tools, "connect", json!({})));
    assert_eq!(choose["error"], "choose_window", "{choose}");
    assert!(
        choose["details"]["chosen_unreadable"].is_string(),
        "{choose}"
    );
    let listed = text(&call(&mut tools, "windows", json!({})));
    assert!(listed["chosen_unreadable"].is_string(), "{listed}");
}

/// MECH-1: a client that gave up on a tool call (Codex stops waiting after
/// 60 s) sends notifications/cancelled. The server stopped waiting for the
/// window only after the call's whole wait, up to 15 minutes, and the window
/// went on working for nobody. Now the call ends at once without a reply, and
/// its connection ends, which tells the window to stop the job.
#[test]
fn a_cancelled_call_stops_waiting_and_lets_the_window_go() {
    let root = tempfile::tempdir().unwrap();
    let events = scripted_window(root.path(), INSTANCE, vec![Ending::Silent]);
    let session = std::sync::Arc::new(Session::new(Tools::new(None, Some(root.path().to_owned()))));
    let status = |id: u64| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
               "params": {"name": "inspect", "arguments": {"action": "status"}}})
    };
    assert_eq!(
        handle(&session, status(1)).unwrap()["result"]["isError"],
        false
    );
    assert_eq!(events.recv().unwrap(), "attach");
    let (send_reply, reply) = mpsc::channel();
    let waiting = std::sync::Arc::clone(&session);
    std::thread::spawn(move || send_reply.send(handle(&waiting, status(7))));
    assert_eq!(events.recv().unwrap(), "waiting");
    let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                        "params": {"requestId": 7, "reason": "timed out"}});
    assert!(handle(&session, cancel).is_none());
    let reply = reply
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the cancelled call ends within 5 s");
    assert!(reply.is_none(), "a cancelled call gets no reply: {reply:?}");
    assert_eq!(
        events.recv_timeout(std::time::Duration::from_secs(5)),
        Ok("ended"),
        "the window sees the connection end"
    );
}

/// MCP-2: after its window closed, the server never moves to another window.
#[test]
fn a_lost_window_is_never_replaced_by_another_open_window() {
    let root = tempfile::tempdir().unwrap();
    let events = scripted_window(root.path(), INSTANCE, vec![Ending::Close]);
    let mut tools = Tools::new(None, Some(root.path().to_owned()));
    let status = json!({"action": "status"});
    assert_eq!(
        call(&mut tools, "inspect", status.clone())["isError"],
        false
    );
    assert_eq!(events.recv().unwrap(), "attach");
    assert_eq!(events.recv().unwrap(), "ended");
    std::fs::remove_file(root.path().join(format!("{INSTANCE}.json"))).unwrap();
    let other = scripted_window(
        root.path(),
        "fedcba9876543210fedcba9876543210",
        vec![Ending::Close],
    );

    let lost = text(&call(&mut tools, "inspect", status.clone()));
    assert_eq!(lost["error"], "connection_lost");
    let message = lost["message"].as_str().unwrap();
    assert!(message.contains("was not sent"), "{message}");
    assert_eq!(
        text(&call(&mut tools, "inspect", status))["error"],
        "window_closed"
    );
    assert!(
        other.try_recv().is_err(),
        "the other window is not attached"
    );
}

/// PR-N2: `open_window` takes a document only by a drive path, so a share or
/// device name never reaches the file system or the new window.
#[cfg(windows)]
#[test]
fn open_window_refuses_a_document_named_through_a_share_or_device_prefix() {
    let root = tempfile::tempdir().unwrap();
    let document = root.path().join("desk.ketchup");
    std::fs::write(&document, b"not opened").unwrap();
    let mut tools = Tools::new(
        Some(root.path().join("missing-Ketchup.exe")),
        Some(root.path().to_owned()),
    );
    for path in [
        format!(r"\\?\{}", document.display()),
        r"\/server.invalid\share\desk.ketchup".to_owned(),
    ] {
        let answer = text(&call(
            &mut tools,
            "open_window",
            json!({"document_path": path}),
        ));
        assert_eq!(answer["error"], "invalid_path", "{path}: {answer}");
    }
}

fn call(tools: &mut Tools, name: &str, arguments: Value) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                         "params": {"name": name, "arguments": arguments}});
    handle(&Session::new(tools), request).unwrap()["result"].clone()
}

fn text(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn a_client_initializes_and_lists_every_tool() {
    let tools = Session::new(Tools::new(None, None));
    let initialized = handle(
        &tools,
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
               "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
    )
    .unwrap();
    assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(initialized["result"]["serverInfo"]["name"], "ketchup");
    let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert!(handle(&tools, notification).is_none());
    let listed = handle(
        &tools,
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
            "drawing",
            "view",
            "batch",
            "run_plugin"
        ]
    );
    let unknown = handle(
        &tools,
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

    // Review 2026-10-17 P3: a reply without the picture is an error, not an
    // image whose data is null.
    let missing = call(
        &mut tools,
        "view",
        json!({"action": "image", "max_side_px": 1}),
    );
    assert_eq!(missing["isError"], true, "{missing}");
    assert_eq!(text(&missing)["error"], "image_missing", "{missing}");
    assert!(
        missing["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|content| content["type"] != "image"),
        "{missing}"
    );
    received.recv().unwrap();
}
