use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantCapability, AssistantDistribution, AssistantHandshake,
};
use ketchup_scheduler::assistant::{AssistantProcessClient, AssistantProcessError};
use serde_json::json;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
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
import sys
import time

mode = sys.argv[1]
hello = json.loads(sys.stdin.readline())
if mode == "exit":
    raise SystemExit(0)
if mode == "timeout":
    time.sleep(30)
    raise SystemExit(0)
if mode == "bad-ready":
    print(json.dumps({"type":"ready","protocol_version":2,"distribution":"public-api","provider":"openai-api","model":hello["model"],"capabilities":hello["capabilities"]}), flush=True)
    time.sleep(30)
    raise SystemExit(0)
print(json.dumps({"type":"ready","protocol_version":hello["protocol_version"],"distribution":hello["distribution"],"provider":hello["provider"],"model":hello["model"],"capabilities":hello["capabilities"]}), flush=True)
request = json.loads(sys.stdin.readline())
if mode == "remote-error":
    print(json.dumps({"type":"error","error":"provider unavailable"}), flush=True)
elif mode == "malformed":
    print("not-json", flush=True)
elif mode == "chat":
    print(json.dumps({"type":"chat-result","request_id":request["request_id"],"message":"bounded answer","model_intent":None}), flush=True)
    shutdown = json.loads(sys.stdin.readline())
    print(json.dumps({"type":"bye"}), flush=True)
elif mode == "chat-timeout":
    time.sleep(30)
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
