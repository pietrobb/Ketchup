//! Bootstrap contract against real pipes/TCP and the actual offscreen app, not OS input.
mod harness;
use harness::Shell;
use ketchup_app::live_bridge::{CaptureMode, Envelope, Request, Response, bootstrap::*};
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
            expected: before.clone(),
            capture_mode: CaptureMode::Offscreen,
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
