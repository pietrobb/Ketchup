//! In-window consent broker for attaching to an already-open Ketchup window.
//!
//! The loopback request carries no credential. A fresh bridge credential is returned
//! over that same socket only after the user approves in this exact app instance.
use super::{KetchupApp, egui, transport};
use crate::dialogs::HighRiskConfirmationRequest;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
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

pub(crate) struct ConsentBroker {
    address: SocketAddr,
    instance_id: String,
    registry_path: PathBuf,
    registry_identity: ketchup_core::persistence::FileIdentity,
    discovery: Arc<Mutex<DiscoveryState>>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    requests: mpsc::Receiver<PendingConsent>,
    deliveries: mpsc::Receiver<bool>,
}

impl Drop for ConsentBroker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
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
}

enum ConsentDecision {
    Allow { address: SocketAddr, token: String },
    Reject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachRequest {
    version: u32,
    action: String,
    nonce: String,
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
}

#[derive(Serialize)]
struct ListResponse<'a> {
    version: u32,
    nonce: &'a str,
    status: &'static str,
    instance_id: &'a str,
    document: &'a str,
}

#[derive(Serialize)]
struct RegistryEntry<'a> {
    version: u32,
    instance_id: &'a str,
    consent_address: String,
}

impl KetchupApp {
    /// Start the nonsecret loopback consent endpoint for this already-open window.
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
        if self.live_consent_broker.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "live consent broker already enabled",
            ));
        }
        let broker = start(
            context.clone(),
            discovery_root,
            self.live_consent_document(),
            self.live_bridge.is_none(),
        )?;
        let address = broker.address;
        self.live_consent_broker = Some(broker);
        self.live_consent_attached = self.live_bridge.is_some();
        Ok(address)
    }

    #[must_use]
    pub fn live_consent_address(&self) -> Option<SocketAddr> {
        self.live_consent_broker
            .as_ref()
            .map(|broker| broker.address)
    }

    #[must_use]
    pub fn live_consent_instance_id(&self) -> Option<&str> {
        self.live_consent_broker
            .as_ref()
            .map(|broker| broker.instance_id.as_str())
    }

    #[must_use]
    pub fn live_consent_attached(&self) -> bool {
        self.live_consent_attached
    }

    /// Local attach requests are granted immediately; the window stays usable and
    /// shows only a passive "connected" panel with a disconnect button.
    pub(crate) fn poll_live_consent(&mut self, context: &egui::Context) {
        let delivery_failed = self.live_consent_broker.as_ref().is_some_and(|broker| {
            let mut failed = false;
            while let Ok(delivered) = broker.deliveries.try_recv() {
                failed |= !delivered;
            }
            failed
        });
        if delivery_failed {
            self.live_consent_attached = false;
            self.disable_live_bridge();
        }
        let document = self.live_consent_document();
        let Some(broker) = self.live_consent_broker.as_ref() else {
            return;
        };
        if let Ok(mut discovery) = broker.discovery.lock() {
            discovery.document = document;
            discovery.available = !self.live_consent_attached;
        }
        let Ok(request) = broker.requests.try_recv() else {
            return;
        };
        if self.live_consent_attached {
            let _ = request.decision.try_send(ConsentDecision::Reject);
        } else {
            self.allow_live_consent(context, request);
        }
    }

    fn allow_live_consent(&mut self, context: &egui::Context, pending: PendingConsent) {
        if self.live_bridge.is_none() {
            let Ok(bridge) = transport::start(context.clone()) else {
                let _ = pending.decision.try_send(ConsentDecision::Reject);
                return;
            };
            self.live_bridge = Some(bridge);
        }
        let Some(credentials) = self.live_bridge_credentials() else {
            let _ = pending.decision.try_send(ConsentDecision::Reject);
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
            self.live_consent_attached = true;
        } else {
            self.disable_live_bridge();
        }
    }

    pub fn revoke_live_consent(&mut self) {
        self.live_consent_attached = false;
        self.disable_live_bridge();
    }

    pub(crate) fn confirm_live_open_path(&mut self, path: &Path) -> bool {
        if !self.live_consent_attached {
            return true;
        }
        let title = self.catalog.text("live-open-consent-title");
        let description = self.catalog.format(
            "live-open-consent-description",
            &std::collections::BTreeMap::from([("path", path.display().to_string())]),
        );
        self.dialogs
            .confirm_high_risk(HighRiskConfirmationRequest {
                title: &title,
                description: &description,
            })
            .is_some()
    }

    fn live_consent_document(&self) -> String {
        self.document_path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_owned)
            .unwrap_or_else(|| self.catalog.text("live-consent-untitled"))
    }

    pub(crate) fn show_live_consent(&mut self, context: &egui::Context) {
        if self.live_consent_attached {
            let mut revoke = false;
            egui::Window::new(self.catalog.text("live-consent-connected-title"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::Vec2::new(-12.0, -44.0))
                .show(context, |ui| {
                    ui.label(self.catalog.text("live-consent-connected-description"));
                    revoke = ui
                        .button(self.catalog.text("live-consent-disconnect"))
                        .clicked();
                });
            if revoke {
                self.revoke_live_consent();
            }
        }
    }
}

fn default_discovery_root() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(unix)]
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
    getrandom::fill(&mut random).map_err(|_| io::Error::other("OS randomness unavailable"))?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn publish_registry_entry(
    root: &Path,
    instance_id: &str,
    address: SocketAddr,
) -> io::Result<(PathBuf, ketchup_core::persistence::FileIdentity)> {
    publish_registry_entry_before_publish(root, instance_id, address, || {})
}

fn publish_registry_entry_before_publish(
    root: &Path,
    instance_id: &str,
    address: SocketAddr,
    before_publish: impl FnOnce(),
) -> io::Result<(PathBuf, ketchup_core::persistence::FileIdentity)> {
    prepare_discovery_root(root)?;
    let path = root.join(format!("{instance_id}.json"));
    let bytes = serde_json::to_vec(&RegistryEntry {
        version: 1,
        instance_id,
        consent_address: address.to_string(),
    })
    .map_err(io::Error::other)?;
    if bytes.len() > MAX_CONSENT_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".ketchup-registry-")
        .tempfile_in(root)?;
    temporary.write_all(&bytes)?;
    temporary.as_file_mut().sync_all()?;
    before_publish();
    temporary
        .persist_noclobber(&path)
        .map_err(|error| error.error)?;
    let identity = ketchup_core::persistence::FileIdentity::from_bytes(&bytes);
    Ok((path, identity))
}

fn remove_registry_entry_if_unchanged(
    path: &Path,
    expected: ketchup_core::persistence::FileIdentity,
) -> io::Result<bool> {
    ketchup_core::persistence::remove_regular_file_if_unchanged(
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
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let instance_id = random_instance_id()?;
    let (registry_path, registry_identity) =
        publish_registry_entry(discovery_root, &instance_id, address)?;
    let discovery = Arc::new(Mutex::new(DiscoveryState {
        document,
        available,
    }));
    let worker_discovery = Arc::clone(&discovery);
    let worker_instance_id = instance_id.clone();
    let (sender, requests) = mpsc::sync_channel(1);
    let (delivery_sender, deliveries) = mpsc::channel();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let worker = match std::thread::Builder::new()
        .name("ketchup-live-consent".into())
        .spawn(move || {
            let mut handlers = Vec::new();
            while !stop.load(Ordering::Acquire) {
                while let Some(index) = handlers.iter().position(JoinHandle::is_finished) {
                    let _ = handlers.swap_remove(index).join();
                }
                match listener.accept() {
                    Ok((stream, peer)) if peer.ip().is_loopback() => {
                        if handlers.len() >= MAX_CONNECTIONS {
                            continue;
                        }
                        let connection_sender = sender.clone();
                        let connection_delivery_sender = delivery_sender.clone();
                        let connection_context = context.clone();
                        let connection_instance_id = worker_instance_id.clone();
                        let connection_discovery = Arc::clone(&worker_discovery);
                        let connection_stop = Arc::clone(&stop);
                        if let Ok(handler) = std::thread::Builder::new()
                            .name("ketchup-live-consent-connection".into())
                            .spawn(move || {
                                serve(
                                    stream,
                                    &connection_sender,
                                    &connection_delivery_sender,
                                    &connection_context,
                                    &connection_instance_id,
                                    &connection_discovery,
                                    &connection_stop,
                                );
                            })
                        {
                            handlers.push(handler);
                        }
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
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
    })
}

fn serve(
    mut stream: TcpStream,
    sender: &mpsc::SyncSender<PendingConsent>,
    delivery_sender: &mpsc::Sender<bool>,
    context: &egui::Context,
    instance_id: &str,
    discovery: &Mutex<DiscoveryState>,
    stop: &AtomicBool,
) {
    let _ = (|| -> io::Result<()> {
        stream.set_nonblocking(false)?;
        let read_deadline = Instant::now() + IO_DEADLINE;
        let request = read_request(&mut stream, read_deadline, stop)?;
        let nonce = request.nonce;
        if request.action == "list" {
            let state = discovery
                .lock()
                .map_err(|_| io::Error::other("discovery state unavailable"))?
                .clone();
            return write_list_response(&mut stream, &nonce, instance_id, &state, stop);
        }
        let (decision, receiver) = mpsc::sync_channel(1);
        if sender.try_send(PendingConsent { decision }).is_err() {
            return write_response(
                &mut stream,
                &nonce,
                instance_id,
                ConsentDecision::Reject,
                stop,
            );
        }
        context.request_repaint();
        stream.set_nonblocking(true)?;
        let decision = wait_for_decision(&stream, &receiver, stop);
        context.request_repaint();
        let decision = decision.inspect_err(|_| {
            // The window granted access to a requester that already left: free it.
            if matches!(receiver.try_recv(), Ok(ConsentDecision::Allow { .. })) {
                let _ = delivery_sender.send(false);
            }
        })?;
        stream.set_nonblocking(false)?;
        let allowed = matches!(decision, ConsentDecision::Allow { .. });
        let result = write_response(&mut stream, &nonce, instance_id, decision, stop);
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
) -> io::Result<AttachRequest> {
    let mut bytes = Vec::with_capacity(MAX_CONSENT_BYTES);
    while bytes.len() < MAX_CONSENT_BYTES {
        if stop.load(Ordering::Acquire) {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(io::ErrorKind::TimedOut)?;
        stream.set_read_timeout(Some(remaining.min(STOP_POLL)))?;
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            let request: AttachRequest =
                serde_json::from_slice(&bytes).map_err(|_| io::ErrorKind::InvalidData)?;
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
    state: &DiscoveryState,
    stop: &AtomicBool,
) -> io::Result<()> {
    write_json_line(
        stream,
        &ListResponse {
            version: 1,
            nonce,
            status: if state.available { "available" } else { "busy" },
            instance_id,
            document: &state.document,
        },
        stop,
    )
}

fn write_response(
    stream: &mut TcpStream,
    nonce: &str,
    instance_id: &str,
    decision: ConsentDecision,
    stop: &AtomicBool,
) -> io::Result<()> {
    let response = match &decision {
        ConsentDecision::Allow { address, token } => AttachResponse {
            version: 1,
            nonce,
            status: "allowed",
            instance_id,
            live_bridge_address: Some(address.to_string()),
            token: Some(token),
        },
        ConsentDecision::Reject => AttachResponse {
            version: 1,
            nonce,
            status: "rejected",
            instance_id,
            live_bridge_address: None,
            token: None,
        },
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

    #[test]
    fn registry_destination_is_invisible_until_complete() {
        let directory = tempfile::tempdir().unwrap();
        let instance_id = "0123456789abcdef0123456789abcdef";
        let path = directory.path().join(format!("{instance_id}.json"));

        publish_registry_entry_before_publish(
            directory.path(),
            instance_id,
            "127.0.0.1:12345".parse().unwrap(),
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
        let directory = tempfile::tempdir().unwrap();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let mut attach = TcpStream::connect(broker.address).unwrap();
        attach
            .write_all(
                br#"{"version":1,"action":"attach","nonce":"0000000000000000000000000000000000000000000000000000000000000000"}
"#,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let pending = loop {
            if let Ok(pending) = broker.requests.try_recv() {
                break pending;
            }
            assert!(Instant::now() < deadline, "attach was not received");
            std::thread::sleep(Duration::from_millis(5));
        };

        let mut list = TcpStream::connect(broker.address).unwrap();
        list.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        list.write_all(
            br#"{"version":1,"action":"list","nonce":"1111111111111111111111111111111111111111111111111111111111111111"}
"#,
        )
        .unwrap();
        let mut response = String::new();
        list.read_to_string(&mut response)
            .expect("a pending consent decision must not block discovery");
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["status"], "available");
        drop(pending);
    }

    #[test]
    fn broker_drop_stops_a_pending_attach_promptly() {
        let directory = tempfile::tempdir().unwrap();
        let broker = start(
            egui::Context::default(),
            directory.path(),
            "Untitled".to_owned(),
            true,
        )
        .unwrap();
        let mut attach = TcpStream::connect(broker.address).unwrap();
        attach
            .write_all(
                br#"{"version":1,"action":"attach","nonce":"2222222222222222222222222222222222222222222222222222222222222222"}
"#,
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let pending = loop {
            if let Ok(pending) = broker.requests.try_recv() {
                break pending;
            }
            assert!(Instant::now() < deadline, "attach was not received");
            std::thread::sleep(Duration::from_millis(5));
        };

        let started = Instant::now();
        drop(broker);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "broker shutdown must not wait for the consent deadline"
        );
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
        let directory = tempfile::tempdir().unwrap();
        let instance_id = "fedcba9876543210fedcba9876543210";
        let path = directory.path().join(format!("{instance_id}.json"));
        fs::write(&path, b"external entry").unwrap();

        let error = publish_registry_entry(
            directory.path(),
            instance_id,
            "127.0.0.1:12345".parse().unwrap(),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"external entry");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
