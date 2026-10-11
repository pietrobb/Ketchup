//! One authenticated connection to a window's live bridge.
//!
//! Wire: u32 big-endian length, then a JSON envelope
//! `{version, id, token, request}`; the reply uses the same framing and is
//! `{version, id, ok, stamp, result, error}`. One request at a time.
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
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
    stream: TcpStream,
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
    /// had reached the window (it may then have been applied).
    Superseded {
        sent: bool,
    },
}

impl Connection {
    pub fn open(address: SocketAddr, token: String) -> io::Result<Self> {
        let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
        stream.set_nodelay(true)?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        Ok(Self {
            stream,
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
        self.stream
            .write_all(&frame)
            .map_err(BridgeError::Transport)?;
        let reply = self.read_reply(wait).map_err(BridgeError::Transport)?;
        if reply["error"] == crate::SUPERSEDED_BY_ANOTHER_CLIENT {
            return Err(BridgeError::Superseded { sent: true });
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
                    Err(BridgeError::Superseded { sent: false })
                }
                Ok(_) => Err(BridgeError::Closed(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unrequested reply",
                ))),
                Err(error) => Err(BridgeError::Closed(error)),
            },
        }
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
        self.stream.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header) as usize;
        if length == 0 || length > MAX_REPLY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reply frame length out of range",
            ));
        }
        let mut body = vec![0_u8; length];
        self.stream.read_exact(&mut body)?;
        Ok(serde_json::from_slice(&body)?)
    }
}
