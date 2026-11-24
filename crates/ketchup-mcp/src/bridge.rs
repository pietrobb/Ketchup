//! One authenticated connection to a window's live bridge.
//!
//! Wire: u32 big-endian length, then a JSON envelope
//! `{version, id, token, request}`; the reply uses the same framing and is
//! `{version, id, ok, stamp, result, error}`. One request at a time.
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

/// The window rejects larger requests (`live_bridge::MAX_REQUEST_FRAME_BYTES`).
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
/// Until a connection has authenticated, the window reads no longer frame:
/// a stranger cannot make it parse megabytes. A larger first request is
/// preceded by a small `status` that authenticates the connection.
pub const MAX_UNAUTHENTICATED_REQUEST_BYTES: usize = 64 * 1024;
/// Images are the largest replies (`live_bridge::MAX_IMAGE_FRAME_BYTES`).
const MAX_REPLY_BYTES: usize = 12 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Connection {
    /// Shared with [`Cancel`], which ends this very socket: on Windows a
    /// duplicate (`try_clone`) of a socket connected with a timeout can stay
    /// "not connected", so ending it ended nothing and a cancelled call
    /// waited for the window to the end.
    stream: Arc<TcpStream>,
    token: String,
    last_id: u64,
    /// The window has answered a request on this connection.
    authenticated: bool,
}

pub enum Reply {
    Done {
        stamp: Value,
        result: Value,
    },
    /// The window refused the request; `result` explains why and how to fix it.
    Rejected {
        code: String,
        result: Value,
    },
}

pub enum BridgeError {
    TooLarge {
        bytes: usize,
    },
    /// The connection failed after the request was written: the window may
    /// or may not have acted on it.
    Transport(io::Error),
    /// The window had already ended the connection; the request was not sent.
    Closed(io::Error),
    /// Another client took the window over; `sent` tells whether this request
    /// had reached the window, `document_changed` whether the document changed
    /// while it ran (`None` from a window that does not say), and `stamp` is
    /// the document at the takeover.
    Superseded {
        sent: bool,
        document_changed: Option<bool>,
        stamp: Value,
    },
}

/// Ends, from another thread, the window connection of a tool call the client
/// cancelled: the call's wait for the window fails at once, and the window,
/// seeing the connection end, stops the job (MECH-1).
#[derive(Clone, Default)]
pub struct Cancel(Arc<Mutex<Armed>>);

#[derive(Default)]
struct Armed {
    cancelled: bool,
    /// The client has gone: every call from now on is cancelled.
    closed: bool,
    /// The connection of the request being waited for.
    stream: Option<Arc<TcpStream>>,
}

impl Cancel {
    /// Cancels the request being waited for, or the next one before it is sent.
    pub fn cancel(&self) -> io::Result<()> {
        let mut armed = self.armed();
        armed.cancelled = true;
        match armed.stream.take() {
            Some(stream) => stream.shutdown(Shutdown::Both),
            None => Ok(()),
        }
    }

    /// Cancels the request being waited for and every one after it, as the
    /// client that asked has gone (review 2026-11-04 r19 MCP-161).
    pub fn close(&self) -> io::Result<()> {
        self.armed().closed = true;
        self.cancel()
    }

    /// Whether the call running now is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.armed().cancelled
    }

    /// A new tool call starts, not cancelled unless the client has gone.
    pub fn clear(&self) {
        let mut armed = self.armed();
        armed.cancelled = armed.closed;
        armed.stream = None;
    }

    /// Lets [`Self::cancel`] end `connection` until [`Self::disarm`]; false
    /// when the call was cancelled already and the request must not be sent.
    pub fn arm(&self, connection: &Connection) -> bool {
        let mut armed = self.armed();
        if armed.cancelled {
            return false;
        }
        armed.stream = Some(Arc::clone(&connection.stream));
        true
    }

    pub fn disarm(&self) {
        self.armed().stream = None;
    }

    fn armed(&self) -> MutexGuard<'_, Armed> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Connection {
    pub fn open(address: SocketAddr, token: String) -> io::Result<Self> {
        let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
        stream.set_nodelay(true)?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        Ok(Self {
            stream: Arc::new(stream),
            token,
            last_id: 0,
            authenticated: false,
        })
    }

    /// Sends `request` (an object with `method`) and waits up to `wait` for the reply.
    pub fn request(&mut self, request: Value, wait: Duration) -> Result<Reply, BridgeError> {
        let mut body = self.envelope(&request);
        if body.len() > MAX_REQUEST_BYTES {
            return Err(BridgeError::TooLarge { bytes: body.len() });
        }
        if !self.authenticated && body.len() > MAX_UNAUTHENTICATED_REQUEST_BYTES {
            if let rejected @ Reply::Rejected { .. } =
                self.request(json!({"method": "status"}), wait)?
            {
                return Ok(rejected);
            }
            body = self.envelope(&request);
        }
        self.ensure_open()?;
        let id = self.last_id + 1;
        self.last_id = id;
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        (&*self.stream)
            .write_all(&frame)
            .map_err(BridgeError::Transport)?;
        let reply = self.read_reply(wait).map_err(BridgeError::Transport)?;
        if reply["error"] == crate::SUPERSEDED_BY_ANOTHER_CLIENT {
            return Err(BridgeError::Superseded {
                sent: true,
                document_changed: reply["result"]["document_changed"].as_bool(),
                stamp: reply["stamp"].clone(),
            });
        }
        if reply["id"] != id {
            return Err(BridgeError::Transport(io::Error::new(
                io::ErrorKind::InvalidData,
                "reply to another request",
            )));
        }
        // Any reply but `unauthorized` (which ends the connection) means the
        // window accepted the token.
        self.authenticated = reply["error"] != "unauthorized";
        let result = reply.get("result").cloned().unwrap_or(Value::Null);
        if reply["ok"] == true {
            let stamp = reply.get("stamp").cloned().unwrap_or(Value::Null);
            return Ok(Reply::Done { stamp, result });
        }
        let code = reply["error"].as_str().unwrap_or("rejected").to_owned();
        Ok(Reply::Rejected { code, result })
    }

    /// Between requests the window sends nothing, except one last frame when
    /// another client takes it over. A pending frame or end of stream is read
    /// before writing, so the request is known not to have been sent.
    fn ensure_open(&mut self) -> Result<(), BridgeError> {
        let mut byte = [0_u8; 1];
        self.stream
            .set_nonblocking(true)
            .map_err(BridgeError::Closed)?;
        let peeked = self.stream.peek(&mut byte);
        self.stream
            .set_nonblocking(false)
            .map_err(BridgeError::Closed)?;
        match peeked {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(BridgeError::Closed(error)),
            Ok(0) => Err(BridgeError::Closed(io::ErrorKind::UnexpectedEof.into())),
            Ok(_) => match self.read_reply(WRITE_TIMEOUT) {
                Ok(reply) if reply["error"] == crate::SUPERSEDED_BY_ANOTHER_CLIENT => {
                    Err(BridgeError::Superseded {
                        sent: false,
                        document_changed: None,
                        stamp: reply["stamp"].clone(),
                    })
                }
                Ok(_) => Err(BridgeError::Closed(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unrequested reply",
                ))),
                Err(error) => Err(BridgeError::Closed(error)),
            },
        }
    }

    /// Blocks up to `wait` until the window has sent something this side can
    /// read (or ended the connection): a test's stand-in window writing a
    /// frame does not mean it has arrived here yet.
    #[cfg(test)]
    pub(crate) fn wait_readable(&self, wait: Duration) -> io::Result<()> {
        self.stream.set_read_timeout(Some(wait))?;
        self.stream.peek(&mut [0_u8; 1]).map(drop)
    }

    /// The framed body of the next request.
    fn envelope(&self, request: &Value) -> Vec<u8> {
        json!({"version": 1, "id": self.last_id + 1, "token": self.token, "request": request})
            .to_string()
            .into_bytes()
    }

    fn read_reply(&mut self, wait: Duration) -> io::Result<Value> {
        self.stream.set_read_timeout(Some(wait))?;
        let mut header = [0_u8; 4];
        (&*self.stream).read_exact(&mut header)?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 || length > MAX_REPLY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reply frame length out of range",
            ));
        }
        let mut body = vec![0_u8; length];
        (&*self.stream).read_exact(&mut body)?;
        Ok(serde_json::from_slice(&body)?)
    }
}
