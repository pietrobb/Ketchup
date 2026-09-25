//! Bootstrap contract against real pipes/TCP and the actual offscreen app, not OS input.
mod harness;
use harness::Shell;
use ketchup_app::{
    AppCommand,
    dialogs::ScriptedFileDialogs,
    live_bridge::{
        CaptureMode, Envelope, IMAGE_PROTOCOL_VERSION, ImageFraming, Request, Response,
        bootstrap::*,
    },
};
use ketchup_core::assistant_sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
};
use ketchup_interaction::Vec3;
use std::{
    ffi::OsString,
    io::{self, Cursor, Read, Write},
    net::TcpStream,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

// Test-only fixed credential; production launchers generate secrets.token_hex(32).
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
fn opted_in() -> LiveStdinBootstrap {
    LiveStdinBootstrap::from_arguments([OsString::from(LIVE_STDIN_FLAG)])
        .unwrap()
        .unwrap()
}
fn line() -> Vec<u8> {
    format!("{{\"version\":1,\"token\":\"{TOKEN}\"}}\n").into_bytes()
}
fn pending() -> PendingBootstrap {
    opted_in().read_from(Cursor::new(line())).unwrap()
}
#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Output {
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().clone()
    }
}

#[test]
fn explicit_flag_and_absolute_optional_path_only() {
    for args in [
        vec![],
        vec!["model.ketchup"],
        vec!["--inspect-native-document", "file"],
        vec!["--supervisor-live-stdin=false"],
    ] {
        assert!(
            LiveStdinBootstrap::from_arguments(args.into_iter().map(OsString::from))
                .unwrap()
                .is_none()
        );
    }
    for args in [
        vec![LIVE_STDIN_FLAG, "relative.ketchup"],
        vec![LIVE_STDIN_FLAG, "--token", TOKEN],
        vec![LIVE_STDIN_FLAG, "a", "b"],
    ] {
        assert!(LiveStdinBootstrap::from_arguments(args.into_iter().map(OsString::from)).is_err());
    }
    let path = std::env::temp_dir().join("bootstrap-test.ketchup");
    assert!(
        LiveStdinBootstrap::from_arguments([
            OsString::from(LIVE_STDIN_FLAG),
            path.into_os_string()
        ])
        .unwrap()
        .is_some()
    );
}

#[test]
fn malformed_oversized_and_invalid_tokens_are_generic_errors() {
    let mut invalid = vec![
        vec![],
        b"not json\n".to_vec(),
        line()[..line().len() - 1].to_vec(),
        format!("{{\"version\":2,\"token\":\"{TOKEN}\"}}\n").into_bytes(),
        format!("{{\"version\":1,\"token\":\"{TOKEN}\",\"extra\":true}}\n").into_bytes(),
        format!("{{\"version\":1,\"version\":1,\"token\":\"{TOKEN}\"}}\n").into_bytes(),
        format!("{{\"version\":1,\"token\":\"{TOKEN}\",\"token\":\"{TOKEN}\"}}\n").into_bytes(),
        b"{\"version\":1}\n".to_vec(),
        vec![0xff, b'\n'],
        [vec![b' '; MAX_BOOTSTRAP_BYTES], line()].concat(),
    ];
    for token in [
        "".to_string(),
        "a".repeat(63),
        "a".repeat(65),
        TOKEN.to_uppercase(),
        "g".repeat(64),
        "é".repeat(32),
    ] {
        invalid.push(format!("{{\"version\":1,\"token\":\"{token}\"}}\n").into_bytes());
    }
    for bytes in invalid {
        let error = opted_in()
            .read_from(Cursor::new(bytes))
            .err()
            .expect("must reject");
        assert_eq!(error.to_string(), "live bridge bootstrap failed");
        assert_eq!(format!("{error:?}"), "BootstrapError");
    }
}

struct CountedReader {
    inner: Cursor<Vec<u8>>,
    count: Arc<AtomicUsize>,
}
impl Read for CountedReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(bytes)?;
        self.count.fetch_add(n, Ordering::SeqCst);
        Ok(n)
    }
}
#[test]
fn consumes_only_one_line_and_never_more_than_1024_bytes() {
    let count = Arc::new(AtomicUsize::new(0));
    let reader = CountedReader {
        inner: Cursor::new([line(), b"not another token\n".to_vec()].concat()),
        count: count.clone(),
    };
    assert!(opted_in().read_from(reader).is_ok());
    assert_eq!(count.load(Ordering::SeqCst), line().len());
    count.store(0, Ordering::SeqCst);
    let reader = CountedReader {
        inner: Cursor::new(vec![b' '; 4096]),
        count: count.clone(),
    };
    assert!(opted_in().read_from(reader).is_err());
    assert_eq!(count.load(Ordering::SeqCst), MAX_BOOTSTRAP_BYTES);
    let mut exact = line();
    exact.pop();
    exact.resize(MAX_BOOTSTRAP_BYTES - 1, b' ');
    exact.push(b'\n');
    assert!(opted_in().read_from(Cursor::new(exact)).is_ok());
}

struct BlockedRead(mpsc::Receiver<()>);
impl Read for BlockedRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        let _ = self.0.recv();
        Ok(0)
    }
}
struct BlockedWrite(mpsc::Receiver<()>);
impl Write for BlockedWrite {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        let _ = self.0.recv();
        Err(io::Error::other("test private detail"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn input_and_output_pipe_stalls_have_deadlines() {
    let (tx, rx) = mpsc::channel();
    let started = Instant::now();
    assert!(opted_in().read_from(BlockedRead(rx)).is_err());
    assert!(started.elapsed() < BOOTSTRAP_DEADLINE + Duration::from_secs(2));
    drop(tx);
    let mut shell = Shell::new();
    let (tx, rx) = mpsc::channel();
    let started = Instant::now();
    assert!(
        pending()
            .enable(
                shell.app_mut(),
                &eframe::egui::Context::default(),
                BlockedWrite(rx)
            )
            .is_err()
    );
    assert!(started.elapsed() < BOOTSTRAP_DEADLINE + Duration::from_secs(2));
    assert!(shell.app().live_bridge_credentials().is_none());
    drop(tx);
}

fn call(shell: &mut Shell, stream: &mut TcpStream, token: &str, request: Request) -> Response {
    let bytes = serde_json::to_vec(&Envelope {
        version: 1,
        id: 1,
        token: token.into(),
        request,
    })
    .unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    let mut reader = stream.try_clone().unwrap();
    reader
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let mut header = [0; 4];
        reader.read_exact(&mut header).unwrap();
        let length = u32::from_be_bytes(header) as usize;
        assert!(length <= ketchup_app::live_bridge::MAX_FRAME_BYTES);
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).unwrap();
        tx.send(serde_json::from_slice::<Response>(&bytes).unwrap())
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    let response = loop {
        if let Ok(response) = rx.try_recv() {
            break response;
        }
        assert!(Instant::now() < deadline);
        shell.step();
        std::thread::sleep(Duration::from_millis(5));
    };
    handle.join().unwrap();
    response
}

#[test]
fn readiness_authentication_and_detach_use_the_actual_app() {
    let mut shell = Shell::new();
    let before = shell.app().live_bridge_stamp();
    assert!(shell.app().live_bridge_credentials().is_none());
    let output = Output::default();
    pending()
        .enable(
            shell.app_mut(),
            &eframe::egui::Context::default(),
            output.clone(),
        )
        .unwrap();
    let bytes = output.bytes();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.ends_with('\n'));
    assert!(!text.contains(TOKEN));
    assert!(!text.contains("token"));
    let ready: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(ready.as_object().unwrap().len(), 2);
    assert_eq!(ready["version"], 1);
    let address = ready["live_bridge_address"].as_str().unwrap();
    assert!(address.starts_with("127.0.0.1:"));
    let mut wrong = TcpStream::connect(address).unwrap();
    let response = call(&mut shell, &mut wrong, &"0".repeat(64), Request::Status {});
    assert_eq!(response.error.as_deref(), Some("unauthorized"));
    drop(wrong);
    let mut stream = TcpStream::connect(address).unwrap();
    let response = call(&mut shell, &mut stream, TOKEN, Request::Status {});
    assert!(response.ok);
    assert_eq!(response.stamp, Some(before.clone()));
    let image = call(
        &mut shell,
        &mut stream,
        TOKEN,
        Request::Image {
            expected: Some(before.clone()),
            image_protocol_version: IMAGE_PROTOCOL_VERSION,
            capture_mode: CaptureMode::Offscreen,
            max_side_px: ketchup_app::live_bridge::MIN_IMAGE_SIDE_PX,
            framing: ImageFraming::Viewport,
            detail_target: None,
        },
    );
    assert!(matches!(
        image.error.as_deref(),
        Some("image_timeout" | "stale_image")
    )); // Async exact publication may invalidate before the unrendered callback times out.
    assert!(call(&mut shell, &mut stream, TOKEN, Request::Disconnect {}).ok);
    drop(stream);
    shell.step();
    assert_eq!(shell.app().live_bridge_stamp(), before);
    assert!(shell.app().live_bridge_credentials().is_some());
    let mut attached = TcpStream::connect(address).unwrap();
    assert!(call(&mut shell, &mut attached, TOKEN, Request::Summary {}).ok);
    assert_eq!(
        output.bytes(),
        bytes,
        "no repeated readiness or token output"
    );
    let duplicate = Output::default();
    assert!(
        pending()
            .enable(
                shell.app_mut(),
                &eframe::egui::Context::default(),
                duplicate.clone()
            )
            .is_err()
    );
    assert!(duplicate.bytes().is_empty());
    assert_eq!(shell.app().live_bridge_stamp(), before);
}

fn request_broker(address: std::net::SocketAddr, action: &str, nonce: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = serde_json::json!({
        "version": 1,
        "action": action,
        "nonce": nonce,
    });
    let mut bytes = serde_json::to_vec(&request).unwrap();
    bytes.push(b'\n');
    stream.write_all(&bytes).unwrap();
    let mut response = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            break;
        }
        response.push(byte[0]);
    }
    serde_json::from_slice(&response).unwrap()
}

fn request_consent(address: std::net::SocketAddr, nonce: &str) -> serde_json::Value {
    request_broker(address, "attach", nonce)
}

fn finish_attach(
    shell: &mut Shell,
    client: std::thread::JoinHandle<serde_json::Value>,
) -> serde_json::Value {
    for _ in 0..200 {
        shell.step();
        if client.is_finished() {
            return client.join().unwrap();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("attach request did not reach the target window");
}

#[test]
fn failed_optional_broker_leaves_manual_cad_available_without_ai_authority() {
    let mut shell = Shell::new();
    let before_revision = shell.app().document_revision();
    let before_occurrences = shell.app().occurrence_count();
    let result = shell.app_mut().enable_live_consent_broker_in(
        &eframe::egui::Context::default(),
        std::path::Path::new("relative-discovery-root"),
    );

    assert!(result.is_err());
    assert!(shell.app().live_consent_address().is_none());
    assert!(shell.app().live_consent_instance_id().is_none());
    assert!(!shell.app().live_consent_attached());
    assert!(shell.app().live_bridge_credentials().is_none());

    assert!(shell.app_mut().create_box());
    assert!(shell.app().document_revision() > before_revision);
    assert_eq!(shell.app().occurrence_count(), before_occurrences + 1);
    assert!(shell.app().live_bridge_credentials().is_none());
}

#[test]
fn bootstrap_window_reconnects_in_place_after_client_loss() {
    let directory = tempfile::tempdir().unwrap();
    let mut shell = Shell::new();
    let before = shell.app().live_bridge_stamp();
    let output = Output::default();
    pending()
        .enable(
            shell.app_mut(),
            &eframe::egui::Context::default(),
            output.clone(),
        )
        .unwrap();
    let original = shell.app().live_bridge_credentials().unwrap();
    let broker = shell
        .app_mut()
        .enable_live_consent_broker_in(&eframe::egui::Context::default(), directory.path())
        .unwrap();
    let instance_id = shell.app().live_consent_instance_id().unwrap().to_owned();
    assert!(
        directory
            .path()
            .join(format!("{instance_id}.json"))
            .is_file()
    );
    assert!(shell.app().live_consent_attached());
    assert_eq!(
        request_broker(broker, "list", &"a".repeat(64))["status"],
        "busy"
    );

    let mut original_client = TcpStream::connect(original.address).unwrap();
    assert!(
        call(
            &mut shell,
            &mut original_client,
            &original.token,
            Request::Status {}
        )
        .ok
    );
    assert!(
        call(
            &mut shell,
            &mut original_client,
            &original.token,
            Request::Disconnect {}
        )
        .ok
    );
    drop(original_client);
    shell.step();
    assert!(shell.app().live_bridge_credentials().is_none());
    assert!(!shell.app().live_consent_attached());
    assert_eq!(
        request_broker(broker, "list", &"b".repeat(64))["status"],
        "available"
    );

    let reconnect = std::thread::spawn(move || request_consent(broker, &"c".repeat(64)));
    let allowed = finish_attach(&mut shell, reconnect);
    assert_eq!(allowed["status"], "allowed");
    let new_token = allowed["token"].as_str().unwrap();
    assert_ne!(new_token, original.token);
    let mut new_client =
        TcpStream::connect(allowed["live_bridge_address"].as_str().unwrap()).unwrap();
    assert_eq!(
        call(&mut shell, &mut new_client, new_token, Request::Status {}).stamp,
        Some(before)
    );
    assert!(shell.app().live_consent_attached());
    assert_eq!(
        request_broker(broker, "list", &"d".repeat(64))["status"],
        "busy"
    );
    let mut stale = TcpStream::connect(allowed["live_bridge_address"].as_str().unwrap()).unwrap();
    assert_eq!(
        call(&mut shell, &mut stale, &original.token, Request::Status {})
            .error
            .as_deref(),
        Some("unauthorized")
    );
    drop(stale);
    new_client.shutdown(std::net::Shutdown::Both).unwrap();
    drop(new_client);
    for _ in 0..100 {
        shell.step();
        if !shell.app().live_consent_attached() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !shell.app().live_consent_attached(),
        "dropped client must free the same window"
    );
    assert_eq!(
        request_broker(broker, "list", &"e".repeat(64))["status"],
        "available"
    );
    assert_eq!(
        output.bytes().iter().filter(|&&byte| byte == b'\n').count(),
        1
    );
}

#[test]
fn per_user_registry_lists_only_nonce_verified_live_window_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let registry_path;
    {
        let mut shell = Shell::new();
        let address = shell
            .app_mut()
            .enable_live_consent_broker_in(&eframe::egui::Context::default(), directory.path())
            .unwrap();
        shell.step();
        let instance_id = shell.app().live_consent_instance_id().unwrap().to_owned();
        let entries: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 1);
        registry_path = entries[0].clone();
        assert_eq!(
            registry_path.file_name().unwrap().to_str().unwrap(),
            format!("{instance_id}.json")
        );
        let bytes = std::fs::read(&registry_path).unwrap();
        assert!(bytes.len() <= 512);
        let entry: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(entry.as_object().unwrap().len(), 3);
        assert_eq!(entry["version"], 1);
        assert_eq!(entry["instance_id"], instance_id);
        assert_eq!(entry["consent_address"], address.to_string());
        assert!(!String::from_utf8(bytes).unwrap().contains("token"));

        let nonce = "3".repeat(64);
        let listed = request_broker(address, "list", &nonce);
        assert_eq!(listed.as_object().unwrap().len(), 5);
        assert_eq!(listed["version"], 1);
        assert_eq!(listed["nonce"], nonce);
        assert_eq!(listed["status"], "available");
        assert_eq!(listed["instance_id"], instance_id);
        assert_eq!(listed["document"], "Untitled");
        assert!(shell.app().live_bridge_credentials().is_none());
    }
    assert!(!registry_path.exists());
}

#[test]
fn consent_broker_cleanup_preserves_an_externally_replaced_registry_entry() {
    let directory = tempfile::tempdir().unwrap();
    let registry_path;
    {
        let mut shell = Shell::new();
        shell
            .app_mut()
            .enable_live_consent_broker_in(&eframe::egui::Context::default(), directory.path())
            .unwrap();
        let instance_id = shell.app().live_consent_instance_id().unwrap();
        registry_path = directory.path().join(format!("{instance_id}.json"));
        std::fs::remove_file(&registry_path).unwrap();
        std::fs::write(&registry_path, b"external replacement").unwrap();
    }

    assert_eq!(
        std::fs::read(&registry_path).unwrap(),
        b"external replacement",
        "broker shutdown must not delete a registry path it no longer owns"
    );
}

#[test]
fn consent_broker_request_deadline_is_cumulative_across_slow_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let mut shell = Shell::new();
    let address = shell
        .app_mut()
        .enable_live_consent_broker_in(&eframe::egui::Context::default(), directory.path())
        .unwrap();
    shell.step();

    let mut slow = TcpStream::connect(address).unwrap();
    slow.write_all(b"{").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let dripper = std::thread::spawn(move || {
        for _ in 0..8 {
            std::thread::sleep(Duration::from_millis(500));
            if slow.write_all(b" ").is_err() {
                break;
            }
        }
    });

    let started = Instant::now();
    let mut prompt = TcpStream::connect(address).unwrap();
    prompt
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    writeln!(
        prompt,
        "{}",
        serde_json::json!({
            "version": 1, "action": "list", "nonce": "6".repeat(64)
        })
    )
    .unwrap();
    let mut response = String::new();
    let read = std::io::BufRead::read_line(&mut std::io::BufReader::new(prompt), &mut response);
    dripper.join().unwrap();

    assert!(
        read.is_ok(),
        "valid request was blocked by a slow peer: {read:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["status"], "available");
    assert_eq!(response["nonce"], "6".repeat(64));
    assert!(!shell.app().live_consent_attached());
}

#[test]
fn consent_broker_accepts_a_request_arriving_after_the_connection() {
    let directory = tempfile::tempdir().unwrap();
    let mut shell = Shell::new();
    let address = shell
        .app_mut()
        .enable_live_consent_broker_in(&eframe::egui::Context::default(), directory.path())
        .unwrap();
    shell.step();
    let before = shell.app().live_bridge_stamp();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    // Let the nonblocking listener accept before any request bytes arrive.
    std::thread::sleep(Duration::from_millis(100));
    let request = serde_json::json!({
        "version": 1, "action": "list", "nonce": "7".repeat(64)
    });
    writeln!(stream, "{request}").unwrap();
    let mut response = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(stream), &mut response).unwrap();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["status"], "available");
    assert_eq!(response["nonce"], "7".repeat(64));
    assert_eq!(shell.app().live_bridge_stamp(), before);
    assert!(!shell.app().live_consent_attached());
}

#[test]
fn malformed_attach_request_with_claimed_identity_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let mut shell = Shell::new();
    let address = shell
        .app_mut()
        .enable_live_consent_broker_in(&eframe::egui::Context::default(), directory.path())
        .unwrap();
    shell.step();

    let mut spoofed = TcpStream::connect(address).unwrap();
    spoofed
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    writeln!(
        spoofed,
        "{}",
        serde_json::json!({
            "version": 1,
            "action": "attach",
            "requester": "Supervisor",
            "nonce": "8".repeat(64),
        })
    )
    .unwrap();
    let mut response = String::new();
    spoofed.read_to_string(&mut response).unwrap();
    shell.step();
    assert!(response.is_empty());
    assert!(!shell.app().live_consent_attached());
    assert!(shell.app().live_bridge_credentials().is_none());
}

#[test]
fn local_attach_is_granted_without_prompt_and_disconnect_revokes_the_credential() {
    let mut shell = Shell::new();
    shell.enable_live_consent_broker();
    let consent_address = shell.app().live_consent_address().unwrap();
    assert!(consent_address.ip().is_loopback());
    assert!(shell.app().live_bridge_credentials().is_none());

    let allowed_nonce = "2".repeat(64);
    let allow_client = std::thread::spawn({
        let allowed_nonce = allowed_nonce.clone();
        move || request_consent(consent_address, &allowed_nonce)
    });
    let allowed = finish_attach(&mut shell, allow_client);
    assert_eq!(allowed["nonce"], allowed_nonce);
    assert_eq!(allowed["status"], "allowed");
    assert_eq!(
        allowed["instance_id"],
        shell.app().live_consent_instance_id().unwrap()
    );
    let token = allowed["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    let address = allowed["live_bridge_address"].as_str().unwrap();
    assert!(address.starts_with("127.0.0.1:"));
    assert!(shell.app().live_consent_attached());
    assert!(shell.has_visible_label(&shell.catalog().text("live-consent-connected-title")));

    // A second requester cannot steal an attached window.
    let second = std::thread::spawn(move || request_consent(consent_address, &"3".repeat(64)));
    let second = finish_attach(&mut shell, second);
    assert_eq!(second["status"], "rejected");
    assert!(second.get("token").is_none());
    assert!(shell.app().live_consent_attached());

    let mut live = TcpStream::connect(address).unwrap();
    assert!(call(&mut shell, &mut live, token, Request::Status {}).ok);
    assert!(call(&mut shell, &mut live, token, Request::Disconnect {}).ok);
    drop(live);
    shell.step();
    assert!(!shell.app().live_consent_attached());
    assert!(shell.app().live_bridge_credentials().is_none());
    assert_eq!(shell.app().live_consent_address(), Some(consent_address));
}

#[test]
fn attached_live_open_requires_explicit_consent_for_the_exact_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("approved-live-open.ketchup");
    let mut source = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&path));
    assert!(source.app_mut().create_box());
    source.click_menu_command("menu-file", AppCommand::SaveAs);
    let target_digest = source.app().canonical_digest();

    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(41)
        .always_discard();
    let probe = dialogs.clone();
    let mut shell = Shell::with_dialogs(dialogs);
    assert!(shell.app_mut().create_box());
    let before_digest = shell.app().canonical_digest();
    shell.enable_live_consent_broker();
    let consent_address = shell.app().live_consent_address().unwrap();
    let attach = std::thread::spawn(move || request_consent(consent_address, &"c".repeat(64)));
    let allowed = finish_attach(&mut shell, attach);
    let token = allowed["token"].as_str().unwrap();
    let mut live = TcpStream::connect(allowed["live_bridge_address"].as_str().unwrap()).unwrap();

    let expected = call(&mut shell, &mut live, token, Request::Status {})
        .stamp
        .unwrap();
    let refused = call(
        &mut shell,
        &mut live,
        token,
        Request::Open {
            expected: Some(expected.clone()),
            path: path.to_string_lossy().into_owned(),
        },
    );
    assert_eq!(refused.error.as_deref(), Some("open_rejected"));
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().document_path().is_none());
    assert_eq!(probe.high_risk_prompts().len(), 1);
    assert!(probe.high_risk_prompts()[0].contains(&path.display().to_string()));
    assert_eq!(probe.discard_prompts(), 0);

    let opened = call(
        &mut shell,
        &mut live,
        token,
        Request::Open {
            expected: Some(expected),
            path: path.to_string_lossy().into_owned(),
        },
    );
    assert!(opened.ok, "{:?}", opened.error);
    assert_eq!(shell.app().document_path(), Some(path.as_path()));
    assert_eq!(shell.app().canonical_digest(), target_digest);
    assert_eq!(probe.high_risk_prompts().len(), 2);
    assert_eq!(probe.discard_prompts(), 1);
}

#[test]
fn disconnected_attach_requester_cannot_leave_window_busy() {
    let mut shell = Shell::new();
    shell.enable_live_consent_broker();
    let consent_address = shell.app().live_consent_address().unwrap();
    let mut abandoned = TcpStream::connect(consent_address).unwrap();
    writeln!(
        abandoned,
        "{}",
        serde_json::json!({
            "version": 1,
            "action": "attach",
            "nonce": "a".repeat(64),
        })
    )
    .unwrap();
    // The requester leaves before the window gets a frame to grant it.
    std::thread::sleep(Duration::from_millis(100));
    abandoned.shutdown(std::net::Shutdown::Both).unwrap();
    drop(abandoned);
    std::thread::sleep(Duration::from_millis(100));

    for _ in 0..20 {
        shell.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!shell.app().live_consent_attached());
    assert!(shell.app().live_bridge_credentials().is_none());

    let client = std::thread::spawn(move || request_consent(consent_address, &"b".repeat(64)));
    assert_eq!(finish_attach(&mut shell, client)["status"], "allowed");
    assert!(shell.app().live_consent_attached());
}

#[test]
fn file_new_preserves_window_live_services_and_invalidates_document_authority() {
    let directory = tempfile::tempdir().unwrap();
    let mut shell = Shell::new();
    let consent_address = shell
        .app_mut()
        .enable_live_consent_broker_in(&eframe::egui::Context::default(), directory.path())
        .unwrap();
    shell.step();
    let instance_id = shell.app().live_consent_instance_id().unwrap().to_owned();
    let registry_path = directory.path().join(format!("{instance_id}.json"));
    assert!(registry_path.is_file());

    let nonce = "4".repeat(64);
    let attach = std::thread::spawn({
        let nonce = nonce.clone();
        move || request_consent(consent_address, &nonce)
    });
    let allowed = finish_attach(&mut shell, attach);
    assert_eq!(allowed["status"], "allowed");
    let token = allowed["token"].as_str().unwrap().to_owned();
    let live_address = allowed["live_bridge_address"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let mut live = TcpStream::connect(live_address).unwrap();

    let before = call(&mut shell, &mut live, &token, Request::Status {})
        .stamp
        .unwrap();
    let proposed = call(
        &mut shell,
        &mut live,
        &token,
        Request::Propose {
            expected: Some(before.clone()),
            selection: Some(vec![]),
            program: AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::SetColor {
                    selector: AssistantCadEntitySelector::Occurrences {
                        occurrence_ids: vec![1],
                    },
                    color: Some([17, 29, 41]),
                }],
            },
        },
    );
    assert!(proposed.ok, "{:?}", proposed.error);
    let proposal_id = proposed.result.unwrap()["proposal_id"].as_u64().unwrap();

    shell.click_menu_command("menu-file", AppCommand::New);
    shell.step();

    let credentials = shell.app().live_bridge_credentials().unwrap();
    assert_eq!(credentials.address, live_address);
    assert_eq!(credentials.token, token);
    assert_eq!(shell.app().live_consent_address(), Some(consent_address));
    assert_eq!(
        shell.app().live_consent_instance_id(),
        Some(instance_id.as_str())
    );
    assert!(shell.app().live_consent_attached());
    assert!(registry_path.is_file());
    let listed = request_broker(consent_address, "list", &"5".repeat(64));
    assert_eq!(listed["status"], "busy");
    assert_eq!(listed["instance_id"], instance_id);

    let status = call(&mut shell, &mut live, &token, Request::Status {});
    assert!(status.ok, "{:?}", status.error);
    let after = status.stamp.unwrap();
    assert_ne!(after.document_id, before.document_id);
    let stale = call(
        &mut shell,
        &mut live,
        &token,
        Request::Commit {
            expected: Some(before),
            proposal_id,
        },
    );
    assert_eq!(stale.error.as_deref(), Some("stale_document"));
    let invalidated = call(
        &mut shell,
        &mut live,
        &token,
        Request::Commit {
            expected: Some(after),
            proposal_id,
        },
    );
    assert_eq!(invalidated.error.as_deref(), Some("proposal_not_found"));
}

#[test]
fn file_open_clears_line_chain_and_measurement_before_live_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("open-interaction-reset.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    let saved_digest = shell.app().canonical_digest();
    let saved_occurrence_count = shell.app().occurrence_count();

    pending()
        .enable(
            shell.app_mut(),
            &eframe::egui::Context::default(),
            Output::default(),
        )
        .unwrap();
    let credentials = shell.app().live_bridge_credentials().unwrap();
    let address = credentials.address;
    let token = credentials.token.clone();
    let mut live = TcpStream::connect(address).unwrap();

    let points = [
        Vec3::new(10.0, 10.0, 20.0),
        Vec3::new(25.0, 10.0, 20.0),
        Vec3::new(25.0, 30.0, 20.0),
    ]
    .map(|point| shell.app().viewport_position(point).unwrap());
    shell.click_command(AppCommand::Line);
    for point in points {
        shell.click_at(point);
    }
    assert_eq!(shell.app().occurrence_count(), saved_occurrence_count + 2);
    shell.click_command(AppCommand::Measure);
    shell.click_at(points[0]);
    shell.click_at(points[1]);
    assert!(shell.app().measured_points().is_some());

    let dirty = call(&mut shell, &mut live, &token, Request::Status {})
        .stamp
        .unwrap();
    let blocked = call(
        &mut shell,
        &mut live,
        &token,
        Request::Propose {
            expected: Some(dirty),
            selection: Some(vec![]),
            program: AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::SetColor {
                    selector: AssistantCadEntitySelector::Occurrences {
                        occurrence_ids: vec![1],
                    },
                    color: Some([17, 29, 41]),
                }],
            },
        },
    );
    assert_eq!(blocked.error.as_deref(), Some("busy"));

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), saved_digest);
    assert_eq!(shell.app().occurrence_count(), saved_occurrence_count);
    assert_eq!(shell.app().measured_points(), None);
    assert!(shell.app().value_input().is_empty());

    let opened = call(&mut shell, &mut live, &token, Request::Status {})
        .stamp
        .unwrap();
    let proposed = call(
        &mut shell,
        &mut live,
        &token,
        Request::Propose {
            expected: Some(opened),
            selection: Some(vec![]),
            program: AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::SetColor {
                    selector: AssistantCadEntitySelector::Occurrences {
                        occurrence_ids: vec![1],
                    },
                    color: Some([17, 29, 41]),
                }],
            },
        },
    );
    assert!(proposed.ok, "{:?}", proposed.error);
}

#[test]
fn unfinished_helix_and_thread_previews_block_live_mutations_without_losing_human_work() {
    for (tool, panel_key, create_key) in [
        (
            AppCommand::Helix,
            "helix-panel-title",
            "action-create-helix",
        ),
        (
            AppCommand::Thread,
            "thread-panel-title",
            "action-create-thread",
        ),
    ] {
        let mut shell = Shell::new();
        pending()
            .enable(
                shell.app_mut(),
                &eframe::egui::Context::default(),
                Output::default(),
            )
            .unwrap();
        let credentials = shell.app().live_bridge_credentials().unwrap();
        let token = credentials.token.clone();
        let mut live = TcpStream::connect(credentials.address).unwrap();
        let expected = call(&mut shell, &mut live, &token, Request::Status {})
            .stamp
            .unwrap();
        let color_program = || AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::SetColor {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: vec![1],
                },
                color: Some([17, 29, 41]),
            }],
        };
        let proposed = call(
            &mut shell,
            &mut live,
            &token,
            Request::Propose {
                expected: Some(expected.clone()),
                selection: Some(vec![]),
                program: color_program(),
            },
        );
        assert!(proposed.ok, "{:?}", proposed.error);
        let proposal_id = proposed.result.unwrap()["proposal_id"].as_u64().unwrap();
        let baseline_revision = shell.app().document_revision();
        let baseline_digest = shell.app().canonical_digest();
        let baseline_undo = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", tool);
        assert!(shell.has_visible_label(&shell.catalog().text(panel_key)));
        assert!(shell.has_visible_label(&shell.catalog().text(create_key)));

        let blocked_commit = call(
            &mut shell,
            &mut live,
            &token,
            Request::Commit {
                expected: Some(expected.clone()),
                proposal_id,
            },
        );
        assert_eq!(blocked_commit.error.as_deref(), Some("busy"));
        let blocked_proposal = call(
            &mut shell,
            &mut live,
            &token,
            Request::Propose {
                expected: Some(expected.clone()),
                selection: Some(vec![]),
                program: color_program(),
            },
        );
        assert_eq!(blocked_proposal.error.as_deref(), Some("busy"));
        let blocked_undo = call(
            &mut shell,
            &mut live,
            &token,
            Request::Undo {
                expected: Some(expected),
            },
        );
        assert_eq!(blocked_undo.error.as_deref(), Some("busy"));

        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo);
        assert!(shell.has_visible_label(&shell.catalog().text(panel_key)));
        assert!(shell.has_visible_label(&shell.catalog().text(create_key)));
        shell.click_button_label(&shell.catalog().text(create_key));
        assert!(shell.app().document_revision() > baseline_revision);
        assert!(!shell.has_visible_label(&shell.catalog().text(panel_key)));
    }
}

#[test]
fn failed_document_open_never_enables_or_emits_readiness() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.ketchup");
    let launch = LiveStdinBootstrap::from_arguments([
        OsString::from(LIVE_STDIN_FLAG),
        path.into_os_string(),
    ])
    .unwrap()
    .unwrap();
    let mut shell = Shell::new();
    let output = Output::default();
    assert!(
        launch
            .read_from(Cursor::new(line()))
            .unwrap()
            .enable(
                shell.app_mut(),
                &eframe::egui::Context::default(),
                output.clone()
            )
            .is_err()
    );
    assert!(shell.app().live_bridge_credentials().is_none());
    assert!(output.bytes().is_empty());
}

#[test]
fn native_invalid_bootstrap_exits_without_window_or_credential_output() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ketchup-app"))
        .arg(LIVE_STDIN_FLAG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{{\"version\":2,\"token\":\"{TOKEN}\"}}\n").as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("native bootstrap hung");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "live bridge bootstrap failed"
    );
}
