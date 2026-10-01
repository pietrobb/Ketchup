//! Explicit trusted-launcher bootstrap shared by native main and offscreen hosts.
//!
//! The launcher must privately pipe stdin and supply one JSON line containing a
//! fresh `secrets.token_hex(32)` token. Never put that token in argv, environment,
//! logs, files, or readiness output. The flag gates the initial pipe;
//! later reconnects use the window's consent broker. Ordinary launch never reads stdin.
//! The launcher must drain stdout; readiness is one nonsecret JSON line. A failed
//! bootstrap is fatal to the launcher startup, not a reason to fall back silently.
use super::{KetchupApp, egui, transport};
use serde::Deserialize;
use std::{
    ffi::{OsStr, OsString},
    fmt,
    io::{self, IsTerminal, Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

pub const LIVE_STDIN_FLAG: &str = "--supervisor-live-stdin";
/// Includes the terminating newline. EOF without a newline is rejected.
pub const MAX_BOOTSTRAP_BYTES: usize = 1024;
pub const BOOTSTRAP_DEADLINE: Duration = Duration::from_secs(2);

/// Names the step that failed. Failures deliberately omit input, credentials, paths and
/// OS messages: an IO failure keeps only its `io::ErrorKind`, a malformed launcher line only
/// its JSON error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapError {
    /// The flag was followed by more than one argument or by a relative document path.
    InvalidArguments,
    /// Stdin is a terminal; the launcher must pipe it.
    InteractiveStdin,
    /// Reading the launcher line failed.
    Read(io::ErrorKind),
    /// The launcher line is not JSON of the bootstrap message shape.
    MalformedMessage(serde_json::error::Category),
    /// The message has an unsupported version or a token that is not 64 lowercase hex digits.
    UnsupportedMessage,
    /// No newline arrived within `MAX_BOOTSTRAP_BYTES`.
    MessageTooLong,
    /// The bootstrap was cancelled before the line was complete.
    Cancelled,
    /// This app already runs a live bridge.
    AlreadyEnabled,
    /// The requested document could not be opened.
    DocumentNotOpened,
    /// The loopback bridge could not start.
    BridgeStart(io::ErrorKind),
    /// Writing the readiness line failed.
    Readiness(io::ErrorKind),
    /// The bounded bootstrap worker could not start.
    Worker(io::ErrorKind),
    /// The bootstrap worker stopped without an answer.
    WorkerStopped,
    /// The step did not finish within `BOOTSTRAP_DEADLINE`.
    DeadlineExceeded,
}

impl fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("live bridge bootstrap failed: ")?;
        match self {
            Self::InvalidArguments => {
                f.write_str("the flag accepts at most one argument, an absolute document path")
            }
            Self::InteractiveStdin => f.write_str("stdin is a terminal; pipe the launcher line"),
            Self::Read(kind) => write!(f, "reading the launcher line failed ({kind})"),
            Self::MalformedMessage(category) => {
                write!(
                    f,
                    "the launcher line is not a bootstrap message ({category:?})"
                )
            }
            Self::UnsupportedMessage => f.write_str(
                "the launcher line needs version 1 and a token of 64 lowercase hex digits",
            ),
            Self::MessageTooLong => write!(
                f,
                "no newline within {MAX_BOOTSTRAP_BYTES} bytes of the launcher line"
            ),
            Self::Cancelled => f.write_str("cancelled before the launcher line was complete"),
            Self::AlreadyEnabled => f.write_str("this window already runs a live bridge"),
            Self::DocumentNotOpened => f.write_str("the requested document could not be opened"),
            Self::BridgeStart(kind) => write!(f, "the loopback bridge could not start ({kind})"),
            Self::Readiness(kind) => write!(f, "writing the readiness line failed ({kind})"),
            Self::Worker(kind) => write!(f, "the bootstrap worker could not start ({kind})"),
            Self::WorkerStopped => f.write_str("the bootstrap worker stopped without an answer"),
            Self::DeadlineExceeded => write!(
                f,
                "the step did not finish within {} s",
                BOOTSTRAP_DEADLINE.as_secs()
            ),
        }
    }
}
impl std::error::Error for BootstrapError {}

/// Cannot be constructed without parsing the explicit launcher flag.
/// Intentionally not Debug/Serialize: startup types must never expose secrets.
pub struct LiveStdinBootstrap {
    document_path: Option<PathBuf>,
}

impl LiveStdinBootstrap {
    /// Arguments exclude the executable. Ordinary launch returns None, untouched.
    /// Flag launch accepts exactly one optional absolute document path.
    pub fn from_arguments(
        arguments: impl IntoIterator<Item = OsString>,
    ) -> Result<Option<Self>, BootstrapError> {
        let mut arguments = arguments.into_iter();
        if arguments.next().as_deref() != Some(OsStr::new(LIVE_STDIN_FLAG)) {
            return Ok(None);
        }
        let document_path = arguments.next().map(PathBuf::from);
        if arguments.next().is_some() || document_path.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err(BootstrapError::InvalidArguments);
        }
        Ok(Some(Self { document_path }))
    }

    /// Only callable after explicit opt-in. Interactive stdin is not supported.
    pub fn read_stdin(self) -> Result<PendingBootstrap, BootstrapError> {
        let stdin = io::stdin();
        if stdin.is_terminal() {
            return Err(BootstrapError::InteractiveStdin);
        }
        self.read_from(stdin)
    }

    /// Same bounded reader used in production and by pipe/headless tests.
    /// An OS read cannot portably be interrupted without unsafe/platform code.
    /// A detached worker bounds the caller's wait; on failure the native caller
    /// exits. The cancellation flag prevents further reads once a blocked read
    /// returns. Do not retry bootstrap in the same process after a timeout.
    pub fn read_from<R: Read + Send + 'static>(
        self,
        mut reader: R,
    ) -> Result<PendingBootstrap, BootstrapError> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&cancelled);
        let result = with_deadline(move || {
            let mut bytes = Vec::with_capacity(MAX_BOOTSTRAP_BYTES);
            for _ in 0..MAX_BOOTSTRAP_BYTES {
                if cancel.load(Ordering::Acquire) {
                    return Err(BootstrapError::Cancelled);
                }
                let mut byte = [0];
                reader
                    .read_exact(&mut byte)
                    .map_err(|error| BootstrapError::Read(error.kind()))?;
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    let message: BootstrapMessage = serde_json::from_slice(&bytes)
                        .map_err(|error| BootstrapError::MalformedMessage(error.classify()))?;
                    if message.version != 1
                        || message.token.len() != 64
                        || !message
                            .token
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    {
                        return Err(BootstrapError::UnsupportedMessage);
                    }
                    return Ok(PendingBootstrap {
                        document_path: self.document_path,
                        token: message.token,
                    });
                }
            }
            Err(BootstrapError::MessageTooLong)
        });
        cancelled.store(true, Ordering::Release);
        result
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapMessage {
    version: u32,
    token: String,
}

/// Validated single-use bootstrap. No token getter, Debug, or Serialize.
pub struct PendingBootstrap {
    document_path: Option<PathBuf>,
    token: String,
}

impl PendingBootstrap {
    /// Opens the requested document on THIS app, enables its normal loopback
    /// bridge, then writes/flushed one nonsecret readiness line. No other store.
    /// Use `std::io::stdout()` in native main; an owned pipe/writer in harnesses.
    /// A failed readiness write disables the bridge; callers must abort startup.
    /// Disconnecting clients later never closes this app or disables its bridge.
    pub fn enable<W: Write + Send + 'static>(
        self,
        app: &mut KetchupApp,
        context: &egui::Context,
        mut readiness: W,
    ) -> Result<(), BootstrapError> {
        if app.live.bridge.is_some() {
            return Err(BootstrapError::AlreadyEnabled);
        }
        if let Some(path) = self.document_path
            && !app.open_document_path(&path)
        {
            return Err(BootstrapError::DocumentNotOpened);
        }
        let bridge = transport::start_with_token(context.clone(), self.token)
            .map_err(|error| BootstrapError::BridgeStart(error.kind()))?;
        let address = bridge.address;
        app.live.bridge = Some(bridge);
        let result = with_deadline(move || {
            // SocketAddr is bound internally to IPv4 loopback; never credential data.
            let line = format!("{{\"version\":1,\"live_bridge_address\":\"{address}\"}}\n");
            readiness
                .write_all(line.as_bytes())
                .map_err(|error| BootstrapError::Readiness(error.kind()))?;
            readiness
                .flush()
                .map_err(|error| BootstrapError::Readiness(error.kind()))
        });
        if result.is_err() {
            app.disable_live_bridge();
        }
        result
    }
}

fn with_deadline<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, BootstrapError> + Send + 'static,
) -> Result<T, BootstrapError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("ketchup-live-bootstrap".into())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .map_err(|error| BootstrapError::Worker(error.kind()))?;
    receiver
        .recv_timeout(BOOTSTRAP_DEADLINE)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => BootstrapError::DeadlineExceeded,
            mpsc::RecvTimeoutError::Disconnected => BootstrapError::WorkerStopped,
        })?
}
