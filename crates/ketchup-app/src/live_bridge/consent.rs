//! In-window consent broker for attaching to an already-open Ketchup window.
//!
//! The loopback request carries no credential. A fresh bridge credential is returned
//! over that same socket only after the user approves in this exact app instance.
use super::{KetchupApp, egui, transport};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

const REQUESTER: &str = "Supervisor";
const MAX_CONSENT_BYTES: usize = 512;
const IO_DEADLINE: Duration = Duration::from_secs(2);
const DECISION_DEADLINE: Duration = Duration::from_secs(60);
const DISCOVERY_DIRECTORY: &str = "Ketchup/live-instances";
const INSTANCE_ID_BYTES: usize = 16;

pub(crate) struct ConsentBroker {
    address: SocketAddr,
    instance_id: String,
    registry_path: PathBuf,
    discovery: Arc<Mutex<DiscoveryState>>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    requests: mpsc::Receiver<PendingConsent>,
}

impl Drop for ConsentBroker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = fs::remove_file(&self.registry_path);
        self.worker.take();
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
    requester: String,
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
        )?;
        let address = broker.address;
        self.live_consent_broker = Some(broker);
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
    pub fn live_consent_pending(&self) -> bool {
        self.live_pending_consent.is_some()
    }

    #[must_use]
    pub fn live_consent_attached(&self) -> bool {
        self.live_consent_attached
    }

    pub(crate) fn poll_live_consent(&mut self) {
        let document = self.live_consent_document();
        let Some(broker) = self.live_consent_broker.as_ref() else {
            return;
        };
        if let Ok(mut discovery) = broker.discovery.lock() {
            discovery.document = document;
            discovery.available =
                self.live_pending_consent.is_none() && !self.live_consent_attached;
        }
        let Ok(request) = broker.requests.try_recv() else {
            return;
        };
        if self.live_pending_consent.is_some() || self.live_consent_attached {
            let _ = request.decision.try_send(ConsentDecision::Reject);
        } else {
            self.live_pending_consent = Some(request);
        }
    }

    fn allow_live_consent(&mut self, context: &egui::Context) {
        let Some(pending) = self.live_pending_consent.take() else {
            return;
        };
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

    fn reject_live_consent(&mut self) {
        if let Some(pending) = self.live_pending_consent.take() {
            let _ = pending.decision.try_send(ConsentDecision::Reject);
        }
    }

    pub fn revoke_live_consent(&mut self) {
        self.reject_live_consent();
        self.live_consent_attached = false;
        self.disable_live_bridge();
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
        if self.live_pending_consent.is_some() {
            let title = self.catalog.text("live-consent-title");
            let description = self.catalog.text("live-consent-description");
            let document = self.live_consent_document();
            let mut allow = false;
            let mut reject = false;
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(context, |ui| {
                    ui.label(description);
                    ui.label(self.catalog.format(
                        "live-consent-document",
                        &std::collections::BTreeMap::from([("document", document)]),
                    ));
                    ui.horizontal(|ui| {
                        allow = ui.button(self.catalog.text("live-consent-allow")).clicked();
                        reject = ui
                            .button(self.catalog.text("live-consent-reject"))
                            .clicked();
                    });
                });
            if allow {
                self.allow_live_consent(context);
            } else if reject {
                self.reject_live_consent();
            }
        } else if self.live_consent_attached {
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
) -> io::Result<PathBuf> {
    prepare_discovery_root(root)?;
    let path = root.join(format!("{instance_id}.json"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    let bytes = serde_json::to_vec(&RegistryEntry {
        version: 1,
        instance_id,
        consent_address: address.to_string(),
    })
    .map_err(io::Error::other)?;
    if bytes.len() > MAX_CONSENT_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

fn start(
    context: egui::Context,
    discovery_root: &Path,
    document: String,
) -> io::Result<ConsentBroker> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let instance_id = random_instance_id()?;
    let registry_path = publish_registry_entry(discovery_root, &instance_id, address)?;
    let discovery = Arc::new(Mutex::new(DiscoveryState {
        document,
        available: true,
    }));
    let worker_discovery = Arc::clone(&discovery);
    let worker_instance_id = instance_id.clone();
    let (sender, requests) = mpsc::sync_channel(1);
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let worker = match std::thread::Builder::new()
        .name("ketchup-live-consent".into())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, peer)) if peer.ip().is_loopback() => {
                        serve(
                            stream,
                            &sender,
                            &context,
                            &worker_instance_id,
                            &worker_discovery,
                        );
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        }) {
        Ok(worker) => worker,
        Err(error) => {
            let _ = fs::remove_file(&registry_path);
            return Err(error);
        }
    };
    Ok(ConsentBroker {
        address,
        instance_id,
        registry_path,
        discovery,
        stopped,
        worker: Some(worker),
        requests,
    })
}

fn serve(
    mut stream: TcpStream,
    sender: &mpsc::SyncSender<PendingConsent>,
    context: &egui::Context,
    instance_id: &str,
    discovery: &Mutex<DiscoveryState>,
) {
    let _ = (|| -> io::Result<()> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(IO_DEADLINE))?;
        stream.set_write_timeout(Some(IO_DEADLINE))?;
        let request = read_request(&mut stream)?;
        let nonce = request.nonce;
        if request.action == "list" {
            let state = discovery
                .lock()
                .map_err(|_| io::Error::other("discovery state unavailable"))?
                .clone();
            return write_list_response(&mut stream, &nonce, instance_id, &state);
        }
        let (decision, receiver) = mpsc::sync_channel(1);
        if sender.try_send(PendingConsent { decision }).is_err() {
            return write_response(&mut stream, &nonce, instance_id, ConsentDecision::Reject);
        }
        context.request_repaint();
        let decision = receiver
            .recv_timeout(DECISION_DEADLINE)
            .unwrap_or(ConsentDecision::Reject);
        write_response(&mut stream, &nonce, instance_id, decision)
    })();
}

fn read_request(stream: &mut TcpStream) -> io::Result<AttachRequest> {
    let mut bytes = Vec::with_capacity(MAX_CONSENT_BYTES);
    for _ in 0..MAX_CONSENT_BYTES {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            let request: AttachRequest =
                serde_json::from_slice(&bytes).map_err(|_| io::ErrorKind::InvalidData)?;
            if request.version != 1
                || !matches!(request.action.as_str(), "list" | "attach")
                || request.requester != REQUESTER
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
    )
}

fn write_response(
    stream: &mut TcpStream,
    nonce: &str,
    instance_id: &str,
    decision: ConsentDecision,
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
    write_json_line(stream, &response)
}

fn write_json_line(stream: &mut TcpStream, value: &impl Serialize) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_CONSENT_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    stream.write_all(&bytes)?;
    stream.flush()
}
