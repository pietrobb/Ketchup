//! In-window consent broker for attaching to an already-open Ketchup window.
//!
//! Attach requires a random bootstrap from the OS-protected per-user registry.
//! Only authenticated requests reach the UI and can replace a previous client.
//! No manual token entry or additional confirmation is needed.
use super::admission::{AUTHENTICATED, EVICTED, PENDING, Pending};
use super::{KetchupApp, egui, transport};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MAX_CONSENT_BYTES: usize = 512;
const IO_DEADLINE: Duration = Duration::from_secs(2);
const STOP_POLL: Duration = Duration::from_millis(25);
const DECISION_DEADLINE: Duration = Duration::from_secs(60);
const MAX_CONNECTIONS: usize = 8;
const DISCOVERY_DIRECTORY: &str = "Ketchup/live-instances";
const INSTANCE_ID_BYTES: usize = 16;
const MAX_CLIENT_NAME_CHARS: usize = 40;

pub(crate) struct ConsentBroker {
    address: SocketAddr,
    instance_id: String,
    registry_path: PathBuf,
    registry_identity: ketchup_model::persistence::FileIdentity,
    discovery: Arc<Mutex<DiscoveryState>>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    requests: mpsc::Receiver<PendingConsent>,
    deliveries: mpsc::Receiver<bool>,
    #[cfg(test)]
    active_connections: Arc<AtomicUsize>,
}

impl Drop for ConsentBroker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        // The worker blocks in accept(); a connection of our own wakes it to
        // see the stop. Without one it would never return, so do not wait.
        if let Some(worker) = self.worker.take()
            && TcpStream::connect(self.address).is_ok()
        {
            let _ = worker.join();
        }
        let _ = remove_registry_entry_if_unchanged(&self.registry_path, self.registry_identity);
    }
}

#[derive(Clone)]
struct DiscoveryState {
    document: String,
    available: bool,
}

pub(crate) struct PendingConsent {
    decision: mpsc::SyncSender<ConsentDecision>,
    /// Set once the requester stopped waiting. The window holds the lock from
    /// the check to the grant, so it never acts on a request nobody waits for.
    abandoned: Arc<Mutex<bool>>,
}

enum ConsentDecision {
    Allow {
        address: SocketAddr,
        token: String,
    },
    Reject,
    /// The user allowed it, but the window could not open its live bridge; the
    /// status line says why. Not a refusal of the client's credential (review
    /// 2026-11-02 of 70d56910).
    Failed,
    /// The window cannot answer now (an earlier attach is pending, or the UI
    /// is held by a long load or a dialog); the credential itself was valid.
    Busy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachRequest {
    version: u32,
    action: String,
    nonce: String,
    /// `local_auth::client_attach_proof`: the client knows the bootstrap,
    /// which itself never crosses the socket.
    #[serde(default)]
    proof: String,
}

#[derive(Serialize)]
struct AttachResponse<'a> {
    version: u32,
    nonce: &'a str,
    status: &'static str,
    instance_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_bridge_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<&'a str>,
    proof: &'a str,
}

#[derive(Serialize)]
struct ListResponse<'a> {
    version: u32,
    nonce: &'a str,
    status: &'static str,
    instance_id: &'a str,
    document: &'a str,
    proof: &'a str,
}

#[derive(Serialize)]
struct RegistryEntry<'a> {
    version: u32,
    instance_id: &'a str,
    consent_address: String,
    bootstrap: &'a str,
}

impl KetchupApp {
    /// Start the bootstrap-authenticated endpoint for this already-open window.
    pub fn enable_live_consent_broker(
        &mut self,
        context: &egui::Context,
    ) -> io::Result<SocketAddr> {
        self.enable_live_consent_broker_in(context, &default_discovery_root()?)
    }

    #[doc(hidden)]
    pub fn enable_live_consent_broker_in(
        &mut self,
        context: &egui::Context,
        discovery_root: &Path,
    ) -> io::Result<SocketAddr> {
        if self.live.consent_broker.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "live consent broker already enabled",
            ));
        }
        let broker = start(
            context.clone(),
            discovery_root,
            self.live_consent_document(),
            self.live.bridge.is_none(),
        )?;
        let address = broker.address;
        self.live.consent_broker = Some(broker);
        self.live.consent_attached = self.live.bridge.is_some();
        Ok(address)
    }

    #[must_use]
    pub fn live_consent_address(&self) -> Option<SocketAddr> {
        self.live
            .consent_broker
            .as_ref()
            .map(|broker| broker.address)
    }

    #[must_use]
    pub fn live_consent_instance_id(&self) -> Option<&str> {
        self.live
            .consent_broker
            .as_ref()
            .map(|broker| broker.instance_id.as_str())
    }

    #[must_use]
    pub fn live_consent_attached(&self) -> bool {
        self.live.consent_attached
    }

    /// Local attach requests are granted immediately; the window stays usable and
    /// its status line names the attached AI next to Disconnect.
    pub(crate) fn poll_live_consent(&mut self, context: &egui::Context) {
        let delivery_failed = self.live.consent_broker.as_ref().is_some_and(|broker| {
            let mut failed = false;
            while let Ok(delivered) = broker.deliveries.try_recv() {
                failed |= !delivered;
            }
            failed
        });
        if delivery_failed {
            self.live.consent_attached = false;
            self.disable_live_bridge();
        }
        let document = self.live_consent_document();
        let Some(broker) = self.live.consent_broker.as_ref() else {
            return;
        };
        if let Ok(mut discovery) = broker.discovery.lock() {
            discovery.document = document;
            discovery.available = !self.live.consent_attached && !self.live.launch_attached;
        }
        let Ok(request) = broker.requests.try_recv() else {
            return;
        };
        self.answer_live_consent(context, request);
    }

    fn answer_live_consent(&mut self, context: &egui::Context, request: PendingConsent) {
        let abandoned = Arc::clone(&request.abandoned);
        let Ok(abandoned) = abandoned.lock() else {
            return;
        };
        if *abandoned {
            // Nobody waits for this grant: the current client keeps the window.
            return;
        }
        if self.live.consent_attached || self.live.launch_attached {
            // The previous client may be gone or forgotten without disconnecting;
            // a new local attach takes over with a fresh credential.
            self.live.consent_attached = false;
            self.live.launch_attached = false;
            self.supersede_live_bridge();
        }
        self.allow_live_consent(context, request);
    }

    fn allow_live_consent(&mut self, context: &egui::Context, pending: PendingConsent) {
        self.allow_live_consent_with(context, pending, transport::start);
    }

    /// Grants `pending` on a bridge `start` opens when the window has none.
    fn allow_live_consent_with(
        &mut self,
        context: &egui::Context,
        pending: PendingConsent,
        start: impl FnOnce(egui::Context) -> io::Result<super::LiveBridge>,
    ) {
        if self.live.bridge.is_none() {
            match start(context.clone()) {
                Ok(bridge) => self.live.bridge = Some(bridge),
                Err(error) => {
                    // The window says why the attach failed; the cause used to be
                    // dropped and the attach refused without a reason (review
                    // 2026-11-01 TST-100).
                    self.digest = self.catalog.format(
                        "live-consent-failed",
                        &std::collections::BTreeMap::from([("reason", error.to_string())]),
                    );
                    let _ = pending.decision.try_send(ConsentDecision::Failed);
                    return;
                }
            }
        }
        let Some(credentials) = self.live_bridge_credentials() else {
            let _ = pending.decision.try_send(ConsentDecision::Failed);
            return;
        };
        if pending
            .decision
            .try_send(ConsentDecision::Allow {
                address: credentials.address,
                token: credentials.token,
            })
            .is_ok()
        {
            self.live.consent_attached = true;
            self.live.client_name = None;
        } else {
            self.disable_live_bridge();
        }
    }

    pub fn revoke_live_consent(&mut self) {
        self.live.consent_attached = false;
        self.disable_live_bridge();
    }

    fn live_consent_document(&self) -> String {
        self.file
            .path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_owned)
            .unwrap_or_else(|| self.catalog.text("live-consent-untitled"))
    }

    /// Connect was clicked here (#2687 point 16): an AI's `connect` without an
    /// id goes to this window, also when several are open.
    pub(crate) fn mark_chosen_window(&self) -> io::Result<()> {
        match &self.live.consent_broker {
            Some(broker) => match broker.registry_path.parent() {
                Some(root) => ketchup_mcp::mark_chosen(root, &broker.instance_id),
                None => Ok(()),
            },
            None => Ok(()),
        }
    }

    /// Whether an AI holds this window, attached here or the one that started it.
    pub(crate) fn ai_attached(&self) -> bool {
        self.live.consent_attached || self.live.launch_attached
    }

    /// Disconnect in the status line: the AI's credential ends, whichever way
    /// it attached.
    pub(crate) fn disconnect_ai(&mut self) {
        self.live.launch_attached = false;
        self.live.client_name = None;
        self.revoke_live_consent();
    }
}

/// `hello`: the attached AI's own name, shown in the status line.
pub(crate) fn hello(app: &mut KetchupApp, client: &str) -> serde_json::Value {
    app.live.client_name = shown_client_name(client);
    serde_json::json!({"shown": app.live.client_name})
}

/// The client's own name as the window shows it: printable, at most
/// [`MAX_CLIENT_NAME_CHARS`] characters, `None` when nothing is left. It is
/// what the client says it is, not a verified identity.
pub(crate) fn shown_client_name(name: &str) -> Option<String> {
    let name: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_CLIENT_NAME_CHARS)
        .collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn default_discovery_root() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    // Same root as `ketchup_mcp`'s discovery.
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("TMPDIR").map(PathBuf::from);
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    #[cfg(not(any(windows, unix)))]
    let base: Option<PathBuf> = None;
    let base = base.filter(|path| path.is_absolute()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "per-user runtime directory unavailable",
        )
    })?;
    Ok(base.join(DISCOVERY_DIRECTORY))
}

fn prepare_discovery_root(root: &Path) -> io::Result<()> {
    if !root.is_absolute() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(root)?;
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(())
}

fn random_instance_id() -> io::Result<String> {
    let mut random = [0_u8; INSTANCE_ID_BYTES];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn publish_registry_entry(
    root: &Path,
    instance_id: &str,
    address: SocketAddr,
    bootstrap: &str,
) -> io::Result<(PathBuf, ketchup_model::persistence::FileIdentity)> {
    publish_registry_entry_before_publish(root, instance_id, address, bootstrap, || {})
}

fn publish_registry_entry_before_publish(
    root: &Path,
    instance_id: &str,
    address: SocketAddr,
    bootstrap: &str,
    before_publish: impl FnOnce(),
) -> io::Result<(PathBuf, ketchup_model::persistence::FileIdentity)> {
    prepare_discovery_root(root)?;
    let path = root.join(format!("{instance_id}.json"));
    let bytes = serde_json::to_vec(&RegistryEntry {
        version: 1,
        instance_id,
        consent_address: address.to_string(),
        bootstrap,
    })
    .map_err(io::Error::other)?;
    if bytes.len() > MAX_CONSENT_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".ketchup-registry-")
        .make_in(root, ketchup_mcp::local_auth::create)?;
    temporary.write_all(&bytes)?;
    temporary.as_file_mut().sync_all()?;
    before_publish();
    temporary
        .persist_noclobber(&path)
        .map_err(|error| error.error)?;
    let identity = ketchup_model::persistence::FileIdentity::from_bytes(&bytes);
    Ok((path, identity))
}

fn remove_registry_entry_if_unchanged(
    path: &Path,
    expected: ketchup_model::persistence::FileIdentity,
) -> io::Result<bool> {
    ketchup_model::persistence::remove_regular_file_if_unchanged(
        path,
        expected,
        MAX_CONSENT_BYTES as u64,
    )
}

fn start(
    context: egui::Context,
    discovery_root: &Path,
    document: String,
    available: bool,
) -> io::Result<ConsentBroker> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    let instance_id = random_instance_id()?;
    let bootstrap = format!("{}{}", random_instance_id()?, random_instance_id()?);
    let (registry_path, registry_identity) =
        publish_registry_entry(discovery_root, &instance_id, address, &bootstrap)?;
    let discovery = Arc::new(Mutex::new(DiscoveryState {
        document,
        available,
    }));
    let worker_discovery = Arc::clone(&discovery);
    // Nonces of attach requests the window was handed: a captured request line
    // replayed later is rejected without asking the user again.
    let used_nonces = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
    let worker_instance_id = instance_id.clone();
    let (sender, requests) = mpsc::sync_channel(1);
    let (delivery_sender, deliveries) = mpsc::channel();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let active_connections = Arc::new(AtomicUsize::new(0));
    let connection_count = Arc::clone(&active_connections);
    let worker = match std::thread::Builder::new()
        .name("ketchup-live-consent".into())
        .spawn(move || {
            let mut handlers = Vec::new();
            let mut pending = Pending::default();
            loop {
                let accepted = listener.accept();
                if stop.load(Ordering::Acquire) {
                    break;
                }
                while let Some(index) = handlers.iter().position(JoinHandle::is_finished) {
                    let _ = handlers.swap_remove(index).join();
                }
                match accepted {
                    Ok((stream, peer)) if peer.ip().is_loopback() => {
                        // Review 2026-10-17 P3: eight idle connections used to
                        // turn every later one away, the bootstrap's holder too.
                        let Some((permit, state)) =
                            pending.admit(&connection_count, MAX_CONNECTIONS)
                        else {
                            continue;
                        };
                        let connection_sender = sender.clone();
                        let connection_delivery_sender = delivery_sender.clone();
                        let connection_context = context.clone();
                        let connection_instance_id = worker_instance_id.clone();
                        let connection_bootstrap = bootstrap.clone();
                        let connection_discovery = Arc::clone(&worker_discovery);
                        let connection_nonces = Arc::clone(&used_nonces);
                        let connection_stop = Arc::clone(&stop);
                        if let Ok(handler) = std::thread::Builder::new()
                            .name("ketchup-live-consent-connection".into())
                            .spawn(move || {
                                let _permit = permit;
                                serve(
                                    stream,
                                    &connection_sender,
                                    &connection_delivery_sender,
                                    &connection_context,
                                    &connection_instance_id,
                                    &connection_bootstrap,
                                    &connection_discovery,
                                    &connection_nonces,
                                    &connection_stop,
                                    &state,
                                );
                            })
                        {
                            handlers.push(handler);
                        }
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            for handler in handlers {
                let _ = handler.join();
            }
        }) {
        Ok(worker) => worker,
        Err(error) => {
            let _ = remove_registry_entry_if_unchanged(&registry_path, registry_identity);
            return Err(error);
        }
    };
    Ok(ConsentBroker {
        address,
        instance_id,
        registry_path,
        registry_identity,
        discovery,
        stopped,
        worker: Some(worker),
        requests,
        deliveries,
        #[cfg(test)]
        active_connections,
    })
}

#[allow(clippy::too_many_arguments)]
fn serve(
    mut stream: TcpStream,
    sender: &mpsc::SyncSender<PendingConsent>,
    delivery_sender: &mpsc::Sender<bool>,
    context: &egui::Context,
    instance_id: &str,
    bootstrap: &str,
    discovery: &Mutex<DiscoveryState>,
    used_nonces: &Mutex<std::collections::HashSet<String>>,
    stop: &AtomicBool,
    state: &AtomicU8,
) {
    let _ = (|| -> io::Result<()> {
        stream.set_nonblocking(false)?;
        let read_deadline = Instant::now() + IO_DEADLINE;
        let request = read_request(&mut stream, read_deadline, stop, state)?;
        let nonce = request.nonce;
        // A connection evicted for a newer one meanwhile stays closed.
        let admit = || {
            state
                .compare_exchange(PENDING, AUTHENTICATED, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        };
        if request.action == "list" {
            // The document name is the user's; only a holder of the bootstrap sees it.
            let expected =
                ketchup_mcp::local_auth::client_list_proof(bootstrap, instance_id, &nonce);
            if !ketchup_mcp::local_auth::proofs_match(&request.proof, &expected) {
                return write_response(
                    &mut stream,
                    &nonce,
                    instance_id,
                    bootstrap,
                    ConsentDecision::Reject,
                    stop,
                );
            }
            if !admit() {
                return Ok(());
            }
            let state = discovery
                .lock()
                .map_err(|_: std::sync::PoisonError<_>| {
                    io::Error::other("discovery state unavailable")
                })?
                .clone();
            return write_list_response(&mut stream, &nonce, instance_id, bootstrap, &state, stop);
        }
        let expected = ketchup_mcp::local_auth::client_attach_proof(bootstrap, instance_id, &nonce);
        if !ketchup_mcp::local_auth::proofs_match(&request.proof, &expected) {
            return write_response(
                &mut stream,
                &nonce,
                instance_id,
                bootstrap,
                ConsentDecision::Reject,
                stop,
            );
        }
        if !admit() {
            return Ok(());
        }
        let (decision, receiver) = mpsc::sync_channel(1);
        let abandoned = Arc::new(Mutex::new(false));
        let pending = PendingConsent {
            decision,
            abandoned: Arc::clone(&abandoned),
        };
        // Only a nonce the window was handed is remembered: a busy answer asks
        // nobody, so a requester answered busy again and again no longer grows
        // the record for as long as the window lives (review 2026-10-16).
        let answer = {
            let mut used = used_nonces.lock().map_err(|_: std::sync::PoisonError<_>| {
                io::Error::other("nonce record unavailable")
            })?;
            if used.contains(&nonce) {
                Some(ConsentDecision::Reject)
            } else if sender.try_send(pending).is_err() {
                Some(ConsentDecision::Busy)
            } else {
                used.insert(nonce.clone());
                None
            }
        };
        if let Some(answer) = answer {
            return write_response(&mut stream, &nonce, instance_id, bootstrap, answer, stop);
        }
        context.request_repaint();
        stream.set_nonblocking(true)?;
        let decision = wait_for_decision(&stream, &receiver, stop);
        context.request_repaint();
        let release_late_grant = || {
            // After this the window drops the request instead of answering it.
            if let Ok(mut abandoned) = abandoned.lock() {
                *abandoned = true;
            }
            // The window granted access to a requester that already left: free it.
            if matches!(receiver.try_recv(), Ok(ConsentDecision::Allow { .. })) {
                let _ = delivery_sender.send(false);
            }
        };
        let decision = match decision {
            // The UI was held too long: answer instead of hanging up, so the
            // client can tell a busy window from a wrong credential.
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                release_late_grant();
                ConsentDecision::Busy
            }
            Err(error) => {
                release_late_grant();
                return Err(error);
            }
            Ok(decision) => decision,
        };
        stream.set_nonblocking(false)?;
        let allowed = matches!(decision, ConsentDecision::Allow { .. });
        let result = write_response(&mut stream, &nonce, instance_id, bootstrap, decision, stop);
        if allowed {
            let _ = delivery_sender.send(result.is_ok());
            context.request_repaint();
        }
        result
    })();
}

fn wait_for_decision(
    stream: &TcpStream,
    receiver: &mpsc::Receiver<ConsentDecision>,
    stop: &AtomicBool,
) -> io::Result<ConsentDecision> {
    let deadline = Instant::now() + DECISION_DEADLINE;
    loop {
        if stop.load(Ordering::Acquire) {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        let mut byte = [0];
        match stream.peek(&mut byte) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(io::ErrorKind::TimedOut)?;
        match receiver.recv_timeout(remaining.min(STOP_POLL)) {
            Ok(decision) => return Ok(decision),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(ConsentDecision::Reject),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn read_request(
    stream: &mut TcpStream,
    deadline: Instant,
    stop: &AtomicBool,
    state: &AtomicU8,
) -> io::Result<AttachRequest> {
    let mut bytes = Vec::with_capacity(MAX_CONSENT_BYTES);
    while bytes.len() < MAX_CONSENT_BYTES {
        let mut byte = [0];
        transport::read_some(stream, &mut byte, Some(deadline), || {
            stop.load(Ordering::Acquire) || state.load(Ordering::Acquire) == EVICTED
        })?;
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            let request: AttachRequest = serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if request.version != 1
                || !matches!(request.action.as_str(), "list" | "attach")
                || !valid_nonce(&request.nonce)
            {
                return Err(io::ErrorKind::InvalidData.into());
            }
            return Ok(request);
        }
    }
    Err(io::ErrorKind::InvalidData.into())
}

fn valid_nonce(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn write_list_response(
    stream: &mut TcpStream,
    nonce: &str,
    instance_id: &str,
    bootstrap: &str,
    state: &DiscoveryState,
    stop: &AtomicBool,
) -> io::Result<()> {
    let status = if state.available { "available" } else { "busy" };
    let proof = ketchup_mcp::local_auth::window_list_proof(
        bootstrap,
        &[instance_id, nonce, status, &state.document],
    );
    write_json_line(
        stream,
        &ListResponse {
            version: 1,
            nonce,
            status,
            instance_id,
            document: &state.document,
            proof: &proof,
        },
        stop,
    )
}

/// Answers an attach with `decision` and the window's proof that it knows the
/// bootstrap, so the client never takes another program for this window.
fn write_response(
    stream: &mut TcpStream,
    nonce: &str,
    instance_id: &str,
    bootstrap: &str,
    decision: ConsentDecision,
    stop: &AtomicBool,
) -> io::Result<()> {
    let (status, live_bridge_address, token) = match &decision {
        ConsentDecision::Allow { address, token } => {
            ("allowed", Some(address.to_string()), Some(token.as_str()))
        }
        ConsentDecision::Busy => ("busy", None, None),
        ConsentDecision::Reject => ("rejected", None, None),
        ConsentDecision::Failed => ("failed", None, None),
    };
    let proof = ketchup_mcp::local_auth::window_attach_proof(
        bootstrap,
        &[
            instance_id,
            nonce,
            status,
            live_bridge_address.as_deref().unwrap_or(""),
            token.unwrap_or(""),
        ],
    );
    let response = AttachResponse {
        version: 1,
        nonce,
        status,
        instance_id,
        live_bridge_address,
        token,
        proof: &proof,
    };
    write_json_line(stream, &response, stop)
}

fn write_json_line(
    stream: &mut TcpStream,
    value: &impl Serialize,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_CONSENT_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let deadline = Instant::now() + IO_DEADLINE;
    let mut remaining_bytes = bytes.as_slice();
    while !remaining_bytes.is_empty() {
        if stop.load(Ordering::Acquire) {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(io::ErrorKind::TimedOut)?;
        stream.set_write_timeout(Some(remaining.min(STOP_POLL)))?;
        match stream.write(remaining_bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => remaining_bytes = &remaining_bytes[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// An attach request as the MCP server sends it: proving it knows the
    /// registry's bootstrap without sending it.
    fn attach_line(registry: &serde_json::Value, nonce: impl AsRef<str>) -> serde_json::Value {
        let nonce = nonce.as_ref();
        let proof = ketchup_mcp::local_auth::client_attach_proof(
            registry["bootstrap"].as_str().unwrap(),
            registry["instance_id"].as_str().unwrap(),
            nonce,
        );
        serde_json::json!({"version": 1, "action": "attach", "nonce": nonce, "proof": proof})
    }

    /// Review 2026-10-17 P3: eight connections that never send a request
    /// turned every later one away until they timed out, and a stranger could
    /// reopen them forever. A newer connection evicts the oldest idle one.
    #[test]
    fn idle_connections_do_not_keep_the_bootstrap_holder_out() {
        let directory = private_tempdir();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let registry: serde_json::Value = serde_json::from_slice(
            &ketchup_mcp::local_auth::read_private(&broker.registry_path).unwrap(),
        )
        .unwrap();
        let idle: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| TcpStream::connect(broker.address).unwrap())
            .collect();
        let deadline = Instant::now() + IO_DEADLINE / 2;
        while broker.active_connections.load(Ordering::Acquire) < MAX_CONNECTIONS {
            assert!(
                Instant::now() < deadline,
                "the idle connections were not taken"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let nonce = "9".repeat(64);
        let proof = ketchup_mcp::local_auth::client_list_proof(
            registry["bootstrap"].as_str().unwrap(),
            registry["instance_id"].as_str().unwrap(),
            &nonce,
        );
        let mut list = TcpStream::connect(broker.address).unwrap();
        // Answered well before the idle connections would time out by themselves.
        list.set_read_timeout(Some(IO_DEADLINE / 2)).unwrap();
        writeln!(
            list,
            "{}",
            serde_json::json!({"version": 1, "action": "list", "nonce": nonce, "proof": proof})
        )
        .unwrap();
        let mut response = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(list), &mut response)
            .expect("the window must answer its bootstrap's holder");
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "available");
        drop(idle);
    }

    /// A discovery root other users can read is refused, as the real per-user
    /// runtime directory is not; a default temp dir on Unix is 0755.
    fn private_tempdir() -> tempfile::TempDir {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tempfile::Builder::new()
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir()
                .unwrap()
        }
        #[cfg(not(unix))]
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn registry_destination_is_invisible_until_complete() {
        let directory = private_tempdir();
        let instance_id = "0123456789abcdef0123456789abcdef";
        let path = directory.path().join(format!("{instance_id}.json"));

        publish_registry_entry_before_publish(
            directory.path(),
            instance_id,
            "127.0.0.1:12345".parse().unwrap(),
            &"a".repeat(64),
            || {
                assert!(
                    !path.exists(),
                    "discovery must not expose the destination before its JSON is complete"
                );
            },
        )
        .unwrap();

        let bytes = fs::read(path).unwrap();
        let entry: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(entry["instance_id"], instance_id);
    }

    #[test]
    fn pending_attach_does_not_block_discovery_requests() {
        let directory = private_tempdir();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let mut attach = TcpStream::connect(broker.address).unwrap();
        let registry: serde_json::Value = serde_json::from_slice(
            &ketchup_mcp::local_auth::read_private(&broker.registry_path).unwrap(),
        )
        .unwrap();
        writeln!(attach, "{}", attach_line(&registry, "0".repeat(64))).unwrap();
        let pending = broker
            .requests
            .recv_timeout(Duration::from_secs(5))
            .expect("attach was not received");

        let mut list = TcpStream::connect(broker.address).unwrap();
        list.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let nonce = "1".repeat(64);
        let proof = ketchup_mcp::local_auth::client_list_proof(
            registry["bootstrap"].as_str().unwrap(),
            registry["instance_id"].as_str().unwrap(),
            &nonce,
        );
        writeln!(
            list,
            "{}",
            serde_json::json!({"version": 1, "action": "list", "nonce": nonce, "proof": proof})
        )
        .unwrap();
        let mut response = String::new();
        list.read_to_string(&mut response)
            .expect("a pending consent decision must not block discovery");
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "available");
        // Review 2026-10-19 P3-C: the list answer proves the window knows the
        // bootstrap, over everything it answers, the document name included.
        assert_eq!(
            response["proof"],
            ketchup_mcp::local_auth::window_list_proof(
                registry["bootstrap"].as_str().unwrap(),
                &[
                    registry["instance_id"].as_str().unwrap(),
                    &nonce,
                    "available",
                    "Untitled"
                ],
            )
        );
        drop(pending);
    }

    #[test]
    fn attach_while_the_window_holds_an_earlier_attach_is_answered_busy_not_rejected() {
        let directory = private_tempdir();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let registry: serde_json::Value = serde_json::from_slice(
            &ketchup_mcp::local_auth::read_private(&broker.registry_path).unwrap(),
        )
        .unwrap();
        let attach = |nonce: &str| {
            let mut stream = TcpStream::connect(broker.address).unwrap();
            writeln!(stream, "{}", attach_line(&registry, nonce)).unwrap();
            stream
        };
        // The UI thread (held by a dialog or a long load) takes neither request:
        // one waits in the window's queue, the other must be answered at once.
        let mut streams = [attach(&"3".repeat(64)), attach(&"4".repeat(64))];
        for stream in &streams {
            stream
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut response = Vec::new();
        'answered: while Instant::now() < deadline {
            for stream in &mut streams {
                let mut buffer = [0; 512];
                match stream.read(&mut buffer) {
                    Ok(count) if count > 0 => {
                        response.extend_from_slice(&buffer[..count]);
                        if response.ends_with(b"\n") {
                            break 'answered;
                        }
                    }
                    _ => {}
                }
            }
        }
        let response: serde_json::Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(response["status"], "busy");
        assert!(response.get("token").is_none());
        // The window proves it knows the bootstrap, bound to this answer.
        let field = |name: &str| response[name].as_str().unwrap_or("").to_owned();
        assert_eq!(
            field("proof"),
            ketchup_mcp::local_auth::window_attach_proof(
                registry["bootstrap"].as_str().unwrap(),
                &[&field("instance_id"), &field("nonce"), "busy", "", ""],
            )
        );
    }

    /// SEC-6: the bootstrap never crosses the socket, and a request that
    /// sends it instead of a proof is not let in.
    #[test]
    fn an_attach_sending_the_bootstrap_itself_is_rejected() {
        let directory = private_tempdir();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let registry: serde_json::Value = serde_json::from_slice(
            &ketchup_mcp::local_auth::read_private(&broker.registry_path).unwrap(),
        )
        .unwrap();
        let mut stream = TcpStream::connect(broker.address).unwrap();
        writeln!(
            stream,
            "{}",
            serde_json::json!({"version": 1, "action": "attach", "nonce": "7".repeat(64),
                               "proof": registry["bootstrap"]})
        )
        .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "rejected");
        assert!(
            broker.requests.try_recv().is_err(),
            "the UI was never asked"
        );
    }

    /// Review 2026-10-15 status: the window did not remember the nonces it
    /// had answered, so an attach request line captured once could be sent
    /// again and ask the user again. A replay is rejected without a prompt.
    #[test]
    fn a_replayed_attach_request_is_rejected_without_asking_again() {
        let directory = private_tempdir();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let registry: serde_json::Value = serde_json::from_slice(
            &ketchup_mcp::local_auth::read_private(&broker.registry_path).unwrap(),
        )
        .unwrap();
        let line = attach_line(&registry, "8".repeat(64));
        let mut first = TcpStream::connect(broker.address).unwrap();
        writeln!(first, "{line}").unwrap();
        let pending = broker
            .requests
            .recv_timeout(Duration::from_secs(5))
            .expect("the first attach asks the user");
        drop(pending);
        first
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut answer = String::new();
        first.read_to_string(&mut answer).unwrap();

        let mut replay = TcpStream::connect(broker.address).unwrap();
        writeln!(replay, "{line}").unwrap();
        replay
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut response = String::new();
        replay.read_to_string(&mut response).unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "rejected");
        assert!(
            broker.requests.try_recv().is_err(),
            "the UI was asked again"
        );
    }

    /// Review 2026-10-16: every attach with a valid proof was remembered, also
    /// one answered busy, which asks nobody, so the record grew for as long as
    /// the window lived. Only a request the window was handed is remembered.
    #[test]
    fn a_nonce_answered_busy_is_not_remembered() {
        let directory = private_tempdir();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let registry: serde_json::Value = serde_json::from_slice(
            &ketchup_mcp::local_auth::read_private(&broker.registry_path).unwrap(),
        )
        .unwrap();
        let attach = |nonce: &str| {
            let mut stream = TcpStream::connect(broker.address).unwrap();
            writeln!(stream, "{}", attach_line(&registry, nonce)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(20)))
                .unwrap();
            stream
        };
        let nonces = ["5".repeat(64), "6".repeat(64)];
        let mut streams = [attach(&nonces[0]), attach(&nonces[1])];
        // One waits in the window's queue, the other is answered busy at once.
        let deadline = Instant::now() + Duration::from_secs(2);
        let busy = 'answered: loop {
            assert!(Instant::now() < deadline, "no attach was answered busy");
            for (index, stream) in streams.iter_mut().enumerate() {
                let mut response = String::new();
                if stream.read_to_string(&mut response).is_ok() || response.ends_with('\n') {
                    let response: serde_json::Value =
                        serde_json::from_str(response.trim()).unwrap();
                    assert_eq!(response["status"], "busy");
                    break 'answered index;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        drop(
            broker
                .requests
                .recv_timeout(Duration::from_secs(5))
                .expect("the other attach asks the user"),
        );
        let _again = attach(&nonces[busy]);
        assert!(
            broker.requests.recv_timeout(Duration::from_secs(5)).is_ok(),
            "the attach answered busy was rejected as a replay"
        );
    }

    /// Review 2026-11-01 TST-100: when the window could not open its bridge, the
    /// cause was dropped and the attach refused without a word in the window.
    #[test]
    fn an_attach_the_bridge_cannot_serve_says_why_in_the_window() {
        let context = egui::Context::default();
        let mut app = KetchupApp::new();
        let (decision, receiver) = mpsc::sync_channel(1);
        let pending = PendingConsent {
            decision,
            abandoned: Arc::new(Mutex::new(false)),
        };
        app.allow_live_consent_with(&context, pending, |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "loopback port blocked by policy",
            ))
        });

        // Review 2026-11-02: the client is told the window failed, not that it
        // refused the client's credential.
        assert!(matches!(receiver.try_recv(), Ok(ConsentDecision::Failed)));
        assert!(!app.live_consent_attached());
        assert!(
            app.digest.contains("loopback port blocked by policy"),
            "the window must say why: {}",
            app.digest
        );
    }

    /// #2687 P1 (points 4, 9, 10): the window names the attached AI as it names
    /// itself, forgets the name when the next one attaches, and Disconnect
    /// ends the credential.
    #[test]
    fn the_window_names_the_attached_ai_until_the_next_attach_or_disconnect() {
        let context = egui::Context::default();
        let mut app = KetchupApp::new();
        let attach = |app: &mut KetchupApp| {
            let (decision, receiver) = mpsc::sync_channel(1);
            let pending = PendingConsent {
                decision,
                abandoned: Arc::new(Mutex::new(false)),
            };
            app.answer_live_consent(&context, pending);
            assert!(matches!(
                receiver.try_recv(),
                Ok(ConsentDecision::Allow { .. })
            ));
        };
        attach(&mut app);
        let hello = |client: &str| super::super::Request::Hello {
            client: client.to_owned(),
        };
        let mut bridge = app.live.bridge.take().unwrap();
        assert_eq!(
            bridge.execute(&mut app, hello(" Claude Code\u{7}\n"), false),
            Ok(serde_json::json!({"shown": "Claude Code"}))
        );
        app.live.bridge = Some(bridge);
        assert_eq!(app.live.client_name.as_deref(), Some("Claude Code"));

        attach(&mut app);
        assert!(app.ai_attached());
        assert_eq!(
            app.live.client_name, None,
            "the next AI did not say its name"
        );

        app.live.client_name = Some("Codex".to_owned());
        app.disconnect_ai();
        assert!(!app.ai_attached());
        assert_eq!(app.live.client_name, None);
        assert!(app.live_bridge_credentials().is_none());
    }

    #[test]
    fn a_client_name_is_shown_printable_and_short_or_not_at_all() {
        assert_eq!(shown_client_name("codex"), Some("codex".to_owned()));
        assert_eq!(shown_client_name(" \u{1b}\t "), None);
        assert_eq!(
            shown_client_name(&"ž".repeat(100)).map(|name| name.chars().count()),
            Some(MAX_CLIENT_NAME_CHARS)
        );
    }

    #[test]
    fn an_abandoned_attach_request_leaves_the_current_client_connected() {
        let directory = private_tempdir();
        let context = egui::Context::default();
        let mut app = KetchupApp::new();
        app.enable_live_consent_broker_in(&context, directory.path())
            .unwrap();
        let broker = app.live.consent_broker.as_ref().unwrap();
        let address = broker.address;
        let registry: serde_json::Value = serde_json::from_slice(
            &ketchup_mcp::local_auth::read_private(&broker.registry_path).unwrap(),
        )
        .unwrap();
        let attach = |nonce: &str| {
            let mut stream = TcpStream::connect(address).unwrap();
            writeln!(stream, "{}", attach_line(&registry, nonce)).unwrap();
            stream
        };
        let take_request = |app: &KetchupApp| {
            app.live
                .consent_broker
                .as_ref()
                .unwrap()
                .requests
                .recv_timeout(Duration::from_secs(5))
                .expect("attach was not received")
        };

        let _current = attach(&"5".repeat(64));
        let request = take_request(&app);
        app.answer_live_consent(&context, request);
        assert!(app.live_consent_attached());
        let current = app.live_bridge_credentials().unwrap();

        // A second requester leaves before the window gets a frame to answer.
        let gone = attach(&"6".repeat(64));
        let request = take_request(&app);
        gone.shutdown(std::net::Shutdown::Both).unwrap();
        drop(gone);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !*request.abandoned.lock().unwrap() {
            assert!(
                Instant::now() < deadline,
                "the broker never saw the requester leave"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        app.answer_live_consent(&context, request);

        assert!(app.live_consent_attached());
        let still = app.live_bridge_credentials().unwrap();
        assert_eq!(
            (still.address, still.token),
            (current.address, current.token),
            "the current client must keep its connection and token"
        );
    }

    #[test]
    fn broker_drop_stops_a_pending_attach_promptly() {
        let directory = private_tempdir();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let mut attach = TcpStream::connect(broker.address).unwrap();
        let registry: serde_json::Value = serde_json::from_slice(
            &ketchup_mcp::local_auth::read_private(&broker.registry_path).unwrap(),
        )
        .unwrap();
        writeln!(attach, "{}", attach_line(&registry, "2".repeat(64))).unwrap();
        let pending = broker
            .requests
            .recv_timeout(Duration::from_secs(5))
            .expect("attach was not received");

        let (dropped, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(broker);
            dropped.send(()).unwrap();
        });
        done.recv_timeout(DECISION_DEADLINE / 2)
            .expect("broker shutdown must not wait for the consent deadline");
        attach
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut byte = [0];
        match attach.read(&mut byte) {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::BrokenPipe
                ) => {}
            other => panic!("shutdown must close without returning credentials: {other:?}"),
        }
        drop(pending);
    }

    #[test]
    fn registry_publish_does_not_clobber_an_existing_destination() {
        let directory = private_tempdir();
        let instance_id = "fedcba9876543210fedcba9876543210";
        let path = directory.path().join(format!("{instance_id}.json"));
        fs::write(&path, b"external entry").unwrap();

        let error = publish_registry_entry(
            directory.path(),
            instance_id,
            "127.0.0.1:12345".parse().unwrap(),
            &"a".repeat(64),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"external entry");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
