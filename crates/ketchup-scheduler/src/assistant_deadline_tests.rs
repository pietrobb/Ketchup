use super::*;
use ketchup_assistant::protocol::PROTOCOL_VERSION;
use ketchup_assistant::sidecar::AssistantCapability;

fn ready_client(mode: &str, timeout: Duration) -> AssistantProcessClient {
    let handshake = AssistantHandshake {
        protocol_version: PROTOCOL_VERSION,
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

#[test]
fn non_utf8_response_line_keeps_the_decoding_position_as_its_cause() {
    let mut reader = std::io::Cursor::new(b"ok\xffbad\n".to_vec());
    let error = read_bounded_line(&mut reader, 64).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let cause = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<std::str::Utf8Error>())
        .expect("the UTF-8 decoding error is the cause");
    assert_eq!(cause.valid_up_to(), 2);
}

#[test]
fn one_response_carries_at_most_one_action() {
    for actions in [
        [false; 3],
        [true, false, false],
        [false, true, false],
        [false, false, true],
    ] {
        assert_eq!(ensure_single_assistant_action(actions), Ok(()));
    }
    for actions in [
        [true, true, false],
        [true, false, true],
        [false, true, true],
        [true; 3],
    ] {
        assert_eq!(
            ensure_single_assistant_action(actions),
            Err(AssistantProcessError::Protocol(
                "assistant returned multiple action programs".to_owned()
            ))
        );
    }
}
