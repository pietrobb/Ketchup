//! Transport owns sockets and bounded messages only, never the GUI/store.
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

const IO_DEADLINE: Duration = Duration::from_secs(2);
const STOP_POLL: Duration = Duration::from_millis(25);

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
    let secret = token.clone();
    let worker = std::thread::Builder::new()
        .name("ketchup-live-bridge".into())
        .spawn(move || {
            let mut session = 0_u64;
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, peer)) if peer.ip().is_loopback() => {
                        let Some(next) = session.checked_add(1) else {
                            break;
                        };
                        session = next;
                        let _ = serve(stream, session, &secret, &sender, &stop, &context);
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
        queue,
        query: ModelQuery::default(),
        observed: None,
        session: 0,
        pending: None,
        next_proposal: 1,
        receipts: VecDeque::new(),
        image: image::ImageState::default(),
    })
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
    let mut bytes = serde_json::to_vec(&response).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME_BYTES {
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
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnDrop(Arc::clone(&cancelled));
    let mut session_authenticated = false;
    while !stop.load(Ordering::Acquire) {
        let bytes = read_frame(&mut stream, session_authenticated, stop)?;
        let envelope: Envelope = match serde_json::from_slice(&bytes) {
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
        let disconnect = matches!(envelope.request, Request::Disconnect {});
        let (reply, receiver) = mpsc::sync_channel(1);
        let queued = Queued {
            session,
            id: envelope.id,
            request: envelope.request,
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
        let deadline = Instant::now() + Duration::from_secs(30);
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
}
