use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantCapability, AssistantDistribution, AssistantHandshake,
};
use ketchup_scheduler::assistant::{
    AssistantCancellation, AssistantProcessClient, AssistantProcessError, AssistantProcessLaunch,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn public_handshake() -> AssistantHandshake {
    AssistantHandshake {
        protocol_version: ASSISTANT_PROTOCOL_VERSION,
        distribution: AssistantDistribution::PublicApi,
        provider: "anthropic-api".to_owned(),
        model: "claude-sonnet-4-6".to_owned(),
        capabilities: BTreeSet::from([
            AssistantCapability::Chat,
            AssistantCapability::LocalMemory,
            AssistantCapability::QueryDocument,
            AssistantCapability::ProposeWorkflowIntent,
        ]),
    }
}

fn write_mock(temp: &TempDir, mode: &str) -> PathBuf {
    let script = temp.path().join(format!("assistant-{mode}.py"));
    fs::write(
        &script,
        r#"import json
import os
import subprocess
import sys
import time

mode = sys.argv[1]
hello = json.loads(sys.stdin.readline())
if mode == "exit":
    raise SystemExit(0)
if mode == "timeout":
    time.sleep(30)
    raise SystemExit(0)
if mode == "descendant-timeout":
    child = 'import pathlib,sys,time; time.sleep(0.4); pathlib.Path(sys.argv[1]).write_text("escaped")'
    subprocess.Popen([sys.executable, '-c', child, sys.argv[2]])
    time.sleep(30)
    raise SystemExit(0)
if mode == "bad-ready":
    print(json.dumps({"type":"ready","protocol_version":2,"distribution":"public-api","provider":"openai-api","model":hello["model"],"capabilities":hello["capabilities"]}), flush=True)
    time.sleep(30)
    raise SystemExit(0)
print(json.dumps({"type":"ready","protocol_version":hello["protocol_version"],"distribution":hello["distribution"],"provider":hello["provider"],"model":hello["model"],"capabilities":hello["capabilities"]}), flush=True)
if mode == "no-read":
    time.sleep(30)
    raise SystemExit(0)
if mode == "slow-read-and-response":
    time.sleep(0.12)
request = json.loads(sys.stdin.readline())
if mode == "remote-error":
    print(json.dumps({"type":"error","error":"provider unavailable"}), flush=True)
elif mode == "malformed":
    print("not-json", flush=True)
elif mode == "chat":
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":"bounded answer","model_intent":None}), flush=True)
    shutdown = json.loads(sys.stdin.readline())
    print(json.dumps({"type":"bye"}), flush=True)
elif mode == "descendant-chat":
    child = 'import pathlib,sys,time; time.sleep(0.4); pathlib.Path(sys.argv[1]).write_text("escaped")'
    subprocess.Popen([sys.executable, '-c', child, sys.argv[2]])
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":"bounded answer","model_intent":None}), flush=True)
    shutdown = json.loads(sys.stdin.readline())
    print(json.dumps({"type":"bye"}), flush=True)
elif mode == "shutdown-error":
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":"bounded answer","model_intent":None}), flush=True)
    shutdown = json.loads(sys.stdin.readline())
    print(json.dumps({"type":"error","error":"shutdown refused"}), flush=True)
elif mode == "isolated":
    observed = json.dumps({"cwd":os.getcwd(),"allowed":os.environ.get("KETCHUP_ALLOWED"),"path":("PATH" in os.environ),"pythonpath":("PYTHONPATH" in os.environ)})
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":observed,"model_intent":None}), flush=True)
    shutdown = json.loads(sys.stdin.readline())
    print(json.dumps({"type":"bye"}), flush=True)
elif mode == "cad-edit":
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":"bounded edit","model_intent":None,"cad_edit_program":{"operations":[{"operation":"copy","selector":{"type":"occurrences","occurrence_ids":[7]},"translation_mm":[10,0,0]}]}}), flush=True)
    shutdown = json.loads(sys.stdin.readline())
    print(json.dumps({"type":"bye"}), flush=True)
elif mode == "cad-edit-unbounded":
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":"unbounded edit","model_intent":None,"cad_edit_program":{"operations":[{"operation":"linear_pattern","selector":{"type":"current_selection"},"instances":7,"step_mm":[1,0,0]}]}}), flush=True)
elif mode == "diagnostics":
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":"observed answer","model_intent":None,"diagnostics":{"provider":"anthropic-api","model":"claude-sonnet-4-6","duration_ms":1250,"input_tokens":1234,"output_tokens":56,"cache_read_tokens":700,"cache_write_tokens":0,"stop_reason":"end_turn","system_prompt":"exact system","request_payload":{"model":"claude-sonnet-4-6","messages":[{"role":"user","content":"hello"}]},"response_text":"observed answer"}}), flush=True)
    shutdown = json.loads(sys.stdin.readline())
    print(json.dumps({"type":"bye"}), flush=True)
elif mode == "chat-timeout":
    time.sleep(30)
elif mode == "slow-read-and-response":
    time.sleep(0.12)
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":"late answer","model_intent":None}), flush=True)
"#,
    )
    .unwrap();
    script
}

fn python() -> &'static str {
    if cfg!(windows) {
        "python.exe"
    } else {
        "python3"
    }
}

fn absolute_python() -> PathBuf {
    let output = Command::new(python())
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .expect("locate Python interpreter");
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .canonicalize()
        .expect("absolute Python interpreter")
}

fn arguments(script: &Path, mode: &str) -> Vec<OsString> {
    vec![script.as_os_str().to_owned(), OsString::from(mode)]
}

#[test]
fn assistant_process_completes_bounded_handshake_chat_and_shutdown() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "chat");
    let mut client = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "chat"),
        public_handshake(),
        Duration::from_secs(10),
    )
    .unwrap();

    let result = client
        .chat("request-1", "Explain this", &json!({"selection": [7]}))
        .unwrap();
    assert_eq!(result.message, "bounded answer");
    assert!(result.model_intent.is_none());
    assert_eq!(client.shutdown(), Ok(()));
    assert_eq!(client.shutdown(), Ok(()));
}

#[test]
fn assistant_process_terminates_when_shutdown_is_refused() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "shutdown-error");
    let mut client = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "shutdown-error"),
        public_handshake(),
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(
        client.chat("request", "hello", &json!({})).unwrap().message,
        "bounded answer"
    );
    assert_eq!(
        client.shutdown(),
        Err(AssistantProcessError::Remote("shutdown refused".to_owned()))
    );
    assert_eq!(client.shutdown(), Ok(()));
}

#[test]
fn assistant_process_isolated_launch_uses_explicit_cwd_and_minimal_environment() {
    let temp = TempDir::new().unwrap();
    let working_directory = temp.path().canonicalize().unwrap();
    let script = write_mock(&temp, "isolated").canonicalize().unwrap();
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut environment = vec![(OsString::from("KETCHUP_ALLOWED"), OsString::from("yes"))];
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SYSTEMROOT") {
        environment.push((OsString::from("SYSTEMROOT"), system_root));
    }
    let executable = absolute_python();
    let launch = AssistantProcessLaunch {
        executable_sha256: ketchup_core::graph::sha256_hex(&fs::read(&executable).unwrap()),
        executable,
        arguments: vec![script.into_os_string(), OsString::from("isolated")],
        working_directory,
        environment,
        environment_files: Vec::new(),
    };
    let mut client = AssistantProcessClient::spawn_isolated_with_cancellation(
        &launch,
        public_handshake(),
        Duration::from_secs(10),
        AssistantCancellation::default(),
    )
    .unwrap();

    let result = client
        .chat("request-isolated", "hello", &json!({}))
        .unwrap();
    let observed: serde_json::Value = serde_json::from_str(&result.message).unwrap();
    assert_eq!(
        PathBuf::from(observed["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        launch.working_directory.canonicalize().unwrap()
    );
    assert_eq!(observed["allowed"], "yes");
    assert_eq!(observed["path"], false);
    assert_eq!(observed["pythonpath"], false);
    assert_eq!(client.shutdown(), Ok(()));

    let mut rejected = launch.clone();
    rejected.executable_sha256 = "0".repeat(64);
    let error = AssistantProcessClient::spawn_isolated_with_cancellation(
        &rejected,
        public_handshake(),
        Duration::from_secs(10),
        AssistantCancellation::default(),
    )
    .unwrap_err();
    assert!(matches!(error, AssistantProcessError::Spawn(_)));
}

#[test]
fn isolated_launch_keeps_environment_files_until_client_drop() {
    let temp = TempDir::new().unwrap();
    let working_directory = temp.path().canonicalize().unwrap();
    let script = write_mock(&temp, "isolated").canonicalize().unwrap();
    let snapshot = tempfile::NamedTempFile::new().unwrap();
    let snapshot_path = snapshot.path().to_path_buf();
    let executable = absolute_python();
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut environment = Vec::new();
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SYSTEMROOT") {
        environment.push((OsString::from("SYSTEMROOT"), system_root));
    }
    let launch = AssistantProcessLaunch {
        executable_sha256: ketchup_core::graph::sha256_hex(&fs::read(&executable).unwrap()),
        executable,
        arguments: vec![script.into_os_string(), OsString::from("isolated")],
        working_directory,
        environment,
        environment_files: vec![Arc::new(snapshot)],
    };
    let mut client = AssistantProcessClient::spawn_isolated_with_cancellation(
        &launch,
        public_handshake(),
        Duration::from_secs(10),
        AssistantCancellation::default(),
    )
    .unwrap();

    drop(launch);
    assert!(snapshot_path.exists());
    client
        .chat("environment-file-lifetime", "hello", &json!({}))
        .unwrap();
    assert_eq!(client.shutdown(), Ok(()));
    drop(client);
    assert!(!snapshot_path.exists());
}

#[test]
fn assistant_process_transports_only_bounded_cad_edit_programs() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "cad-edit");
    let mut client = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "cad-edit"),
        public_handshake(),
        Duration::from_secs(10),
    )
    .unwrap();

    let exchange = client
        .chat_exchange("request-cad", "Copy occurrence 7", &json!({}))
        .unwrap();
    assert!(exchange.result.model_intent.is_none());
    assert_eq!(exchange.cad_edit_program.unwrap().operations.len(), 1);
    assert_eq!(client.shutdown(), Ok(()));

    let invalid_temp = TempDir::new().unwrap();
    let invalid_script = write_mock(&invalid_temp, "cad-edit-unbounded");
    let mut invalid_client = AssistantProcessClient::spawn(
        python(),
        &arguments(&invalid_script, "cad-edit-unbounded"),
        public_handshake(),
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(matches!(
        invalid_client.chat_exchange("request-unbounded", "Pattern selection", &json!({})),
        Err(AssistantProcessError::Protocol(_))
    ));
    assert_eq!(
        invalid_client.chat("request-after-rejection", "hello", &json!({})),
        Err(AssistantProcessError::Closed)
    );
}

#[test]
fn assistant_process_returns_bounded_exact_api_diagnostics_when_requested() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "diagnostics");
    let mut handshake = public_handshake();
    handshake
        .capabilities
        .insert(AssistantCapability::DebugObservability);
    let mut client = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "diagnostics"),
        handshake,
        Duration::from_secs(10),
    )
    .unwrap();

    let exchange = client
        .chat_exchange("request-observed", "hello", &json!({}))
        .unwrap();
    assert_eq!(exchange.result.message, "observed answer");
    let diagnostics = exchange.diagnostics.unwrap();
    assert_eq!(diagnostics.input_tokens, 1_234);
    assert_eq!(diagnostics.output_tokens, 56);
    assert_eq!(diagnostics.cache_read_tokens, 700);
    assert_eq!(diagnostics.total_tokens(), 1_290);
    assert_eq!(diagnostics.system_prompt, "exact system");
    assert_eq!(
        diagnostics.request_payload["messages"][0]["content"],
        "hello"
    );
    assert_eq!(client.shutdown(), Ok(()));
}

#[test]
fn assistant_process_rejects_mismatched_ready_and_kills_the_child() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "bad-ready");
    let error = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "bad-ready"),
        public_handshake(),
        Duration::from_secs(10),
    )
    .unwrap_err();

    assert!(matches!(error, AssistantProcessError::Protocol(_)));
}

#[test]
fn assistant_process_surfaces_remote_and_malformed_responses_fail_closed() {
    for (mode, expected) in [("remote-error", "remote"), ("malformed", "protocol")] {
        let temp = TempDir::new().unwrap();
        let script = write_mock(&temp, mode);
        let mut client = AssistantProcessClient::spawn(
            python(),
            &arguments(&script, mode),
            public_handshake(),
            Duration::from_secs(10),
        )
        .unwrap();
        let error = client.chat("request", "hello", &json!({})).unwrap_err();
        match expected {
            "remote" => assert_eq!(
                error,
                AssistantProcessError::Remote("provider unavailable".to_owned())
            ),
            _ => assert!(matches!(error, AssistantProcessError::Protocol(_))),
        }
    }
}

#[test]
fn assistant_process_rejects_unrepresentable_timeout_before_spawn() {
    let error = AssistantProcessClient::spawn(
        PathBuf::from("assistant-must-not-be-spawned"),
        &[],
        public_handshake(),
        Duration::MAX,
    )
    .unwrap_err();

    assert_eq!(error, AssistantProcessError::InvalidTimeout);
}

#[test]
fn assistant_process_times_out_and_terminates_during_handshake_or_chat() {
    for mode in ["timeout", "chat-timeout"] {
        let temp = TempDir::new().unwrap();
        let script = write_mock(&temp, mode);
        if mode == "timeout" {
            let error = AssistantProcessClient::spawn(
                python(),
                &arguments(&script, mode),
                public_handshake(),
                Duration::from_millis(100),
            )
            .unwrap_err();
            assert_eq!(error, AssistantProcessError::TimedOut);
        } else {
            let mut client = AssistantProcessClient::spawn(
                python(),
                &arguments(&script, mode),
                public_handshake(),
                Duration::from_millis(100),
            )
            .unwrap();
            assert_eq!(
                client.chat("request", "hello", &json!({})),
                Err(AssistantProcessError::TimedOut)
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn assistant_process_timeout_terminates_sidecar_descendants() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "descendant-timeout");
    let sentinel = temp.path().join("escaped-descendant.txt");
    let error = AssistantProcessClient::spawn(
        python(),
        &[
            script.into_os_string(),
            OsString::from("descendant-timeout"),
            sentinel.as_os_str().to_owned(),
        ],
        public_handshake(),
        Duration::from_millis(100),
    )
    .unwrap_err();

    assert_eq!(error, AssistantProcessError::TimedOut);
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !sentinel.exists(),
        "assistant descendant survived host timeout and performed a delayed side effect"
    );
}

#[cfg(windows)]
#[test]
fn assistant_process_shutdown_terminates_sidecar_descendants() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "descendant-chat");
    let sentinel = temp.path().join("escaped-after-shutdown.txt");
    let mut client = AssistantProcessClient::spawn(
        python(),
        &[
            script.into_os_string(),
            OsString::from("descendant-chat"),
            sentinel.as_os_str().to_owned(),
        ],
        public_handshake(),
        Duration::from_secs(10),
    )
    .unwrap();

    client.chat("request", "hello", &json!({})).unwrap();
    assert_eq!(client.shutdown(), Ok(()));
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !sentinel.exists(),
        "assistant descendant survived successful shutdown"
    );
}

#[test]
fn assistant_process_times_out_when_the_sidecar_stops_reading_requests() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "no-read");
    let mut client = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "no-read"),
        public_handshake(),
        Duration::from_secs(1),
    )
    .unwrap();
    let message = "x".repeat(120 * 1024);
    assert_eq!(
        client.chat("request", &message, &json!({})),
        Err(AssistantProcessError::TimedOut)
    );
}

#[test]
fn assistant_process_chat_uses_one_cumulative_io_deadline() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "slow-read-and-response");
    let mut client = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "slow-read-and-response"),
        public_handshake(),
        Duration::from_millis(180),
    )
    .unwrap();
    let message = "x".repeat(120 * 1024);

    let started = Instant::now();
    assert_eq!(
        client.chat("request", &message, &json!({})),
        Err(AssistantProcessError::TimedOut)
    );
    assert!(started.elapsed() < Duration::from_millis(300));
}

#[test]
fn assistant_process_rejects_exit_and_invalid_local_handshake() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "exit");
    let error = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "exit"),
        public_handshake(),
        Duration::from_secs(10),
    )
    .unwrap_err();
    assert_eq!(error, AssistantProcessError::Exited);

    let mut invalid = public_handshake();
    invalid.provider = "arbitrary-provider".to_owned();
    let error = AssistantProcessClient::spawn(
        python(),
        &arguments(&script, "chat"),
        invalid,
        Duration::from_secs(10),
    )
    .unwrap_err();
    assert!(matches!(error, AssistantProcessError::Protocol(_)));
}

#[test]
fn assistant_process_cancel_is_observed_before_spawned_io_can_complete() {
    let temp = TempDir::new().unwrap();
    let script = write_mock(&temp, "chat-timeout");
    let cancellation = ketchup_scheduler::assistant::AssistantCancellation::default();
    let mut client = AssistantProcessClient::spawn_with_cancellation(
        python(),
        &arguments(&script, "chat-timeout"),
        public_handshake(),
        Duration::from_secs(10),
        cancellation.clone(),
    )
    .unwrap();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        cancellation.cancel();
    });
    assert_eq!(
        client.chat("request", "hello", &json!({})),
        Err(AssistantProcessError::Cancelled)
    );
    canceller.join().unwrap();
}

#[test]
fn assistant_module_never_links_document_store_or_command_mutation() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/assistant.rs")).unwrap();
    for forbidden in [
        "DocumentStore",
        "CommandBatch",
        "WorkflowIntent",
        "propose_intent",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden authority in process client: {forbidden}"
        );
    }

    let output = Command::new(python())
        .args(["-c", "print('ok')"])
        .output()
        .unwrap();
    assert!(output.status.success());
}
