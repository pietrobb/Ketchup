use super::*;
use ketchup_assistant::sidecar::{ASSISTANT_PROTOCOL_VERSION, AssistantCapability};

fn ready_client(mode: &str, timeout: Duration) -> AssistantProcessClient {
    let handshake = AssistantHandshake {
        protocol_version: ASSISTANT_PROTOCOL_VERSION,
        distribution: AssistantDistribution::PublicApi,
        provider: "anthropic-api".to_owned(),
        model: "claude-sonnet-4-6".to_owned(),
        capabilities: BTreeSet::from([AssistantCapability::Chat]),
    };
    let script = r#"import json,sys,time
hello = json.loads(sys.stdin.readline())
hello['type'] = 'ready'
print(json.dumps(hello), flush=True)
first = sys.stdin.read(1)
if sys.argv[1] == 'cumulative':
    time.sleep(0.12)
request = json.loads(first + sys.stdin.readline())
if sys.argv[1] == 'cumulative':
    time.sleep(0.12)
    print(json.dumps({'type':'chat-result','request_id':request['request_id'],'message':'late answer','model_intent':None}), flush=True)
else:
    time.sleep(30)
"#;
    let mut client = AssistantProcessClient::spawn(
        if cfg!(windows) {
            "python.exe"
        } else {
            "python3"
        },
        &[
            OsString::from("-c"),
            OsString::from(script),
            OsString::from(mode),
        ],
        handshake,
        Duration::from_secs(10),
    )
    .unwrap();
    // Startup is not the operation under test. Keep the original chat deadline,
    // but arm it only after the real subprocess has acknowledged the handshake.
    client.timeout = timeout;
    client
}

#[test]
fn chat_timeout_terminates_after_ready_handshake() {
    let mut client = ready_client("timeout", Duration::from_millis(100));
    let started = Instant::now();
    assert_eq!(
        client.chat("request", "hello", &serde_json::json!({})),
        Err(AssistantProcessError::TimedOut)
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(client.closed);
    client.child.inner_mut().wait().unwrap();
}

#[test]
fn chat_uses_one_cumulative_io_deadline_after_ready_handshake() {
    let mut client = ready_client("cumulative", Duration::from_millis(180));
    let message = "x".repeat(120 * 1024);
    let started = Instant::now();
    assert_eq!(
        client.chat("request", &message, &serde_json::json!({})),
        Err(AssistantProcessError::TimedOut)
    );
    assert!(started.elapsed() >= Duration::from_millis(180));
    assert!(started.elapsed() < Duration::from_millis(300));
    assert!(client.closed);
    client.child.inner_mut().wait().unwrap();
}
