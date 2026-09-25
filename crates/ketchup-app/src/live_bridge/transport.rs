//! Transport owns sockets and bounded messages only, never the GUI/store.
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

const IO_DEADLINE: Duration = Duration::from_secs(2);
const STOP_POLL: Duration = Duration::from_millis(25);
const MAX_CONNECTIONS: usize = 4;

struct ConnectionPermit(Arc<AtomicUsize>);

impl ConnectionPermit {
    fn claim(active: &Arc<AtomicUsize>) -> Option<Self> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_CONNECTIONS).then_some(count + 1)
            })
            .ok()
            .map(|_| Self(Arc::clone(active)))
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) fn start(context: egui::Context) -> io::Result<LiveBridge> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| io::Error::other("OS randomness unavailable"))?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    start_with_token(context, token)
}
pub(super) fn start_with_token(context: egui::Context, token: String) -> io::Result<LiveBridge> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let (sender, queue) = mpsc::sync_channel(QUEUE_CAPACITY);
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let active_connections = Arc::new(AtomicUsize::new(0));
    let connection_count = Arc::clone(&active_connections);
    let secret = token.clone();
    let worker = std::thread::Builder::new()
        .name("ketchup-live-bridge".into())
        .spawn(move || {
            let mut session = 0_u64;
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, peer)) if peer.ip().is_loopback() => {
                        let Some(permit) = ConnectionPermit::claim(&connection_count) else {
                            continue;
                        };
                        let Some(next) = session.checked_add(1) else {
                            break;
                        };
                        session = next;
                        let secret = secret.clone();
                        let sender = sender.clone();
                        let stop = Arc::clone(&stop);
                        let context = context.clone();
                        let authenticated = Arc::new(AtomicBool::new(false));
                        let _ = std::thread::Builder::new()
                            .name(format!("ketchup-live-client-{session}"))
                            .spawn(move || {
                                let _permit = permit;
                                let _ = serve(
                                    stream,
                                    session,
                                    &secret,
                                    &sender,
                                    &stop,
                                    &context,
                                    &authenticated,
                                );
                                if authenticated.load(Ordering::Acquire) {
                                    let (reply, _receiver) = mpsc::sync_channel(1);
                                    let _ = sender.try_send(Queued {
                                        session,
                                        id: 0,
                                        request: Request::Disconnect {},
                                        connection_closed: true,
                                        cancelled: Arc::new(AtomicBool::new(false)),
                                        reply,
                                    });
                                    context.request_repaint();
                                }
                            });
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        })?;
    Ok(LiveBridge {
        address,
        token,
        stopped,
        worker: Some(worker),
        #[cfg(test)]
        active_connections,
        queue,
        query: ModelQuery::default(),
        observed: None,
        session: 0,
        client_states: BTreeMap::new(),
        pending: None,
        next_proposal: 1,
        apply_and_verify_job: None,
        #[cfg(test)]
        apply_and_verify_fault: None,
        batch_jobs: VecDeque::new(),
        batch_job_key: RandomState::new(),
        next_batch_job: 1,
        image: image::ImageState::default(),
    })
}

/// Envelope whose request body is parsed only after authentication, so a
/// malformed body can be answered on the same connection with its id.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnvelope {
    version: u32,
    id: u64,
    token: String,
    request: Value,
}

const DEFAULT_RESPONSE_WAIT: Duration = Duration::from_secs(30);
/// Queueing and publishing on the UI thread come on top of the job deadline.
const APPLY_AND_VERIFY_RESPONSE_MARGIN: Duration = Duration::from_secs(15);

/// How long the connection waits for the UI thread's answer. A verified edit
/// may legitimately run up to its own `timeout_ms`, so the connection must not
/// give up (and cancel it) before the job's deadline does.
fn response_wait(request: &Request) -> Duration {
    match request {
        Request::ApplyAndVerify { timeout_ms, .. } => (Duration::from_millis(*timeout_ms)
            + APPLY_AND_VERIFY_RESPONSE_MARGIN)
            .max(DEFAULT_RESPONSE_WAIT),
        _ => DEFAULT_RESPONSE_WAIT,
    }
}

fn authenticated(supplied: &str, expected: &str) -> bool {
    if supplied.len() != 64 {
        return false;
    }
    supplied
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "frame timeout"))
}

fn read_until(
    stream: &mut TcpStream,
    mut buffer: &mut [u8],
    deadline: Option<Instant>,
    stop: &AtomicBool,
) -> io::Result<()> {
    while !buffer.is_empty() {
        if stop.load(Ordering::Acquire) {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        let timeout = deadline
            .map(remaining)
            .transpose()?
            .unwrap_or(STOP_POLL)
            .min(STOP_POLL);
        stream.set_read_timeout(Some(timeout))?;
        match stream.read(buffer) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => buffer = &mut buffer[count..],
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    if let Some(deadline) = deadline {
        remaining(deadline)?;
    }
    Ok(())
}

fn read_frame(
    stream: &mut TcpStream,
    session_authenticated: bool,
    stop: &AtomicBool,
) -> io::Result<Vec<u8>> {
    // Only an authenticated client may think indefinitely, and only BEFORE
    // the first byte. Header and body then share one absolute frame deadline.
    let pre_auth_deadline = (!session_authenticated).then(|| Instant::now() + IO_DEADLINE);
    let mut header = [0; 4];
    read_until(stream, &mut header[..1], pre_auth_deadline, stop)?;
    let deadline = pre_auth_deadline.unwrap_or_else(|| Instant::now() + IO_DEADLINE);
    read_until(stream, &mut header[1..], Some(deadline), stop)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut bytes = vec![0; length];
    read_until(stream, &mut bytes, Some(deadline), stop)?;
    Ok(bytes)
}

// The closure performs ONE write with the supplied remaining budget. Keeping
// deadline accounting here also permits deterministic controlled-partial-IO tests.
fn write_frame_until(
    bytes: &[u8],
    deadline: Instant,
    mut write: impl FnMut(&[u8], Duration) -> io::Result<usize>,
) -> io::Result<()> {
    let header = (bytes.len() as u32).to_be_bytes();
    for mut buffer in [header.as_slice(), bytes] {
        while !buffer.is_empty() {
            let budget = remaining(deadline)?;
            match write(buffer, budget) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => buffer = &buffer[count..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
            remaining(deadline)?;
        }
    }
    Ok(())
}

fn write_response(stream: &mut TcpStream, response: Response) -> io::Result<()> {
    let limit = if response
        .result
        .as_ref()
        .and_then(|result| result.get("scope"))
        .and_then(Value::as_str)
        == Some("cad_viewport")
    {
        MAX_IMAGE_FRAME_BYTES
    } else {
        MAX_FRAME_BYTES
    };
    let mut bytes = serde_json::to_vec(&response).map_err(io::Error::other)?;
    if bytes.len() > limit {
        bytes = serde_json::to_vec(&Response::error(response.id, "response_limit"))
            .map_err(io::Error::other)?;
    }
    write_frame_until(&bytes, Instant::now() + IO_DEADLINE, |buffer, budget| {
        stream.set_write_timeout(Some(budget))?;
        stream.write(buffer)
    })
}

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn serve(
    mut stream: TcpStream,
    session: u64,
    secret: &str,
    sender: &mpsc::SyncSender<Queued>,
    stop: &AtomicBool,
    context: &egui::Context,
    authenticated_session: &AtomicBool,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnDrop(Arc::clone(&cancelled));
    let mut session_authenticated = false;
    while !stop.load(Ordering::Acquire) {
        let bytes = read_frame(&mut stream, session_authenticated, stop)?;
        let envelope: RawEnvelope = match serde_json::from_slice(&bytes) {
            Ok(envelope) => envelope,
            Err(_) => {
                write_response(&mut stream, Response::error(0, "invalid_request"))?;
                return Ok(());
            }
        };
        if !authenticated(&envelope.token, secret) {
            write_response(&mut stream, Response::error(envelope.id, "unauthorized"))?;
            return Ok(());
        }
        if envelope.version != 1 {
            write_response(
                &mut stream,
                Response::error(envelope.id, "unsupported_version"),
            )?;
            return Ok(());
        }
        session_authenticated = true;
        authenticated_session.store(true, Ordering::Release);
        // A malformed request body from an authenticated client is a fixable
        // argument mistake, not a reason to drop the connection.
        let envelope = match serde_json::from_value::<Request>(envelope.request) {
            Ok(request) => Envelope {
                version: envelope.version,
                id: envelope.id,
                token: envelope.token,
                request,
            },
            Err(error) => {
                write_response(
                    &mut stream,
                    Response::invalid_params(envelope.id, &error.to_string()),
                )?;
                continue;
            }
        };
        let disconnect = matches!(envelope.request, Request::Disconnect {});
        let response_wait = response_wait(&envelope.request);
        let (reply, receiver) = mpsc::sync_channel(1);
        let queued = Queued {
            session,
            id: envelope.id,
            request: envelope.request,
            connection_closed: false,
            cancelled: Arc::clone(&cancelled),
            reply,
        };
        if sender.try_send(queued).is_err() {
            write_response(
                &mut stream,
                Response::error(envelope.id, "queue_unavailable"),
            )?;
            return Ok(());
        }
        context.request_repaint();
        let deadline = Instant::now() + response_wait;
        stream.set_nonblocking(true)?;
        let response = loop {
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Ok(());
            }
            let mut byte = [0];
            match stream.peek(&mut byte) {
                // EOF revokes queued authority. Positive bytes are forbidden
                // pipelining, not a reason to leave a queued mutation alive.
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(response) => break response,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        };
        stream.set_nonblocking(false)?;
        write_response(&mut stream, response)?;
        if disconnect {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for_connection_count(bridge: &LiveBridge, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while bridge.active_connections.load(Ordering::Acquire) != expected {
            assert!(
                Instant::now() < deadline,
                "connection registry did not reach {expected}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn hundred_connection_cycles_and_limit_release_every_permit() {
        let bridge = start_with_token(egui::Context::default(), "a".repeat(64)).unwrap();
        for _ in 0..100 {
            let stream = TcpStream::connect(bridge.address).unwrap();
            wait_for_connection_count(&bridge, 1);
            drop(stream);
            wait_for_connection_count(&bridge, 0);
        }

        let live: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| TcpStream::connect(bridge.address).unwrap())
            .collect();
        wait_for_connection_count(&bridge, MAX_CONNECTIONS);
        let mut refused = TcpStream::connect(bridge.address).unwrap();
        refused
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(refused.read(&mut byte).unwrap(), 0);
        assert_eq!(
            bridge.active_connections.load(Ordering::Acquire),
            MAX_CONNECTIONS
        );

        drop(live);
        wait_for_connection_count(&bridge, 0);
    }

    #[test]
    fn cumulative_partial_write_deadline_includes_header_and_body() {
        let budget = Duration::from_millis(120);
        let mut budgets = Vec::new();
        let mut written = Vec::new();
        let error = write_frame_until(b"body", Instant::now() + budget, |bytes, remaining| {
            budgets.push(remaining);
            // Header is one partial write, then successive one-byte body writes.
            let count = if written.is_empty() { 4 } else { 1 };
            std::thread::sleep(Duration::from_millis(45));
            written.extend_from_slice(&bytes[..count]);
            Ok(count)
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(written.len() < 8);
        assert!(budgets.len() >= 2);
        assert!(budgets.windows(2).all(|pair| pair[1] < pair[0]));
        let mut complete = Vec::new();
        write_frame_until(b"body", Instant::now() + IO_DEADLINE, |bytes, _| {
            complete.push(bytes[0]);
            Ok(1)
        })
        .unwrap();
        assert_eq!(complete, [0, 0, 0, 4, b'b', b'o', b'd', b'y']);
    }

    fn exchange(stream: &mut TcpStream, value: &Value) -> Value {
        let body = serde_json::to_vec(value).unwrap();
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(&body).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut header = [0; 4];
        stream.read_exact(&mut header).unwrap();
        let mut response = vec![0; u32::from_be_bytes(header) as usize];
        stream.read_exact(&mut response).unwrap();
        serde_json::from_slice(&response).unwrap()
    }

    #[test]
    fn malformed_request_body_is_answered_with_its_id_and_keeps_the_connection() {
        let token = "a".repeat(64);
        let bridge = start_with_token(egui::Context::default(), token.clone()).unwrap();
        let mut stream = TcpStream::connect(bridge.address).unwrap();
        let answer = exchange(
            &mut stream,
            &json!({"version":1,"id":7,"token":token,"request":{"method":"no_such_method"}}),
        );
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["ok"], false);
        assert_eq!(answer["error"], "invalid_params");
        let reason = answer["result"]["reason"].as_str().unwrap();
        assert!(reason.contains("no_such_method"), "{reason}");

        // The same connection still accepts a well-formed request.
        let body = serde_json::to_vec(
            &json!({"version":1,"id":8,"token":token,"request":{"method":"status"}}),
        )
        .unwrap();
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(&body).unwrap();
        let queued = bridge
            .queue
            .recv_timeout(Duration::from_secs(2))
            .expect("the well-formed request reaches the UI queue");
        assert_eq!(queued.id, 8);
        assert!(matches!(queued.request, Request::Status { .. }));
    }

    #[test]
    fn apply_and_verify_response_wait_covers_its_own_job_deadline() {
        let long: Request = serde_json::from_value(json!({
            "method": "apply_and_verify",
            "program": {"operations": []},
            "timeout_ms": 120_000,
        }))
        .unwrap();
        assert_eq!(response_wait(&long), Duration::from_secs(135));
        let short: Request = serde_json::from_value(json!({
            "method": "apply_and_verify",
            "program": {"operations": []},
            "timeout_ms": 1_000,
        }))
        .unwrap();
        assert_eq!(response_wait(&short), DEFAULT_RESPONSE_WAIT);
        let status: Request = serde_json::from_value(json!({"method": "status"})).unwrap();
        assert_eq!(response_wait(&status), DEFAULT_RESPONSE_WAIT);
    }
}
