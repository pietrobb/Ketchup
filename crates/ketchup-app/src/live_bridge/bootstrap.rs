//! Explicit trusted-launcher bootstrap shared by native main and offscreen hosts.
//!
//! The launcher must privately pipe stdin and supply one JSON line containing a
//! fresh `secrets.token_hex(32)` token. Never put that token in argv, environment,
//! logs, files, or readiness output. The explicit flag is the trust boundary;
//! this is not endpoint discovery. No stdin is acquired/read for ordinary launch.
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

/// All failures deliberately omit input, credentials, paths, and underlying IO errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootstrapError;
impl fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("live bridge bootstrap failed")
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
            return Err(BootstrapError);
        }
        Ok(Some(Self { document_path }))
    }

    /// Only callable after explicit opt-in. Interactive stdin is not supported.
    pub fn read_stdin(self) -> Result<PendingBootstrap, BootstrapError> {
        let stdin = io::stdin();
        if stdin.is_terminal() {
            return Err(BootstrapError);
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
                    return Err(BootstrapError);
                }
                let mut byte = [0];
                reader.read_exact(&mut byte).map_err(|_| BootstrapError)?;
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    let message: BootstrapMessage =
                        serde_json::from_slice(&bytes).map_err(|_| BootstrapError)?;
                    if message.version != 1
                        || message.token.len() != 64
                        || !message
                            .token
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    {
                        return Err(BootstrapError);
                    }
                    return Ok(PendingBootstrap {
                        document_path: self.document_path,
                        token: message.token,
                    });
                }
            }
            Err(BootstrapError)
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
        if app.live_bridge.is_some() {
            return Err(BootstrapError);
        }
        if let Some(path) = self.document_path {
            if !app.open_document_path(&path) {
                return Err(BootstrapError);
            }
        }
        let bridge =
            transport::start_with_token(context.clone(), self.token).map_err(|_| BootstrapError)?;
        let address = bridge.address;
        app.live_bridge = Some(bridge);
        let result = with_deadline(move || {
            // SocketAddr is bound internally to IPv4 loopback; never credential data.
            let line = format!("{{\"version\":1,\"live_bridge_address\":\"{address}\"}}\n");
            readiness
                .write_all(line.as_bytes())
                .map_err(|_| BootstrapError)?;
            readiness.flush().map_err(|_| BootstrapError)
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
        .map_err(|_| BootstrapError)?;
    receiver
        .recv_timeout(BOOTSTRAP_DEADLINE)
        .map_err(|_| BootstrapError)?
}
