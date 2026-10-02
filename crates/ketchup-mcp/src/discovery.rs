//! Finding open Kečup windows and attaching to one.
//!
//! Every window publishes `<instance_id>.json` with its loopback attach address
//! in the per-user discovery directory. One line of JSON asks the window to
//! `list` itself (document name, whether a client is attached) or to `attach`,
//! which returns the live bridge address and a fresh credential.
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

const DISCOVERY_DIRECTORY: &str = "Ketchup/live-instances";
const MAX_LINE_BYTES: u64 = 512;
const MAX_REGISTRY_ENTRIES: usize = 4096;
const MAX_WINDOWS: usize = 64;
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
/// A window grants a local attach at once; this only bounds a hung window.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(65);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub instance_id: String,
    pub document: String,
    /// Another client is attached; attaching replaces it.
    pub in_use: bool,
    attach_address: SocketAddr,
    registry_path: PathBuf,
}

pub struct Grant {
    pub address: SocketAddr,
    pub token: String,
}

/// `%LOCALAPPDATA%\Ketchup\live-instances` on Windows, `$XDG_RUNTIME_DIR/...` elsewhere.
pub fn default_root() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA");
    #[cfg(unix)]
    let base = std::env::var_os("XDG_RUNTIME_DIR");
    #[cfg(not(any(windows, unix)))]
    let base: Option<std::ffi::OsString> = None;
    base.map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join(DISCOVERY_DIRECTORY))
}

/// Windows that answer on their attach endpoint; stale entries are skipped.
pub fn list_windows(root: &Path) -> Vec<Window> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let registered: Vec<(String, SocketAddr)> = entries
        .flatten()
        .take(MAX_REGISTRY_ENTRIES)
        .filter_map(|entry| registry_entry(&entry.path()))
        .take(MAX_WINDOWS)
        .collect();
    // Entries of crashed windows stay behind, and a refused loopback connect
    // takes about a second on Windows: probe all at once.
    let mut windows: Vec<Window> = std::thread::scope(|scope| {
        let probes: Vec<_> = registered
            .iter()
            .map(|(instance_id, address)| scope.spawn(move || probe(root, instance_id, *address)))
            .collect();
        probes
            .into_iter()
            .filter_map(|probe| probe.join().ok()?.ok())
            .collect()
    });
    windows.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
    windows
}

/// Readiness comes exclusively from the spawned child's private stdout pipe.
pub(crate) fn launched_window(
    reader: impl Read,
    document: String,
) -> io::Result<(Window, SocketAddr)> {
    let mut bytes = Vec::new();
    BufReader::new(reader.take(MAX_LINE_BYTES)).read_until(b'\n', &mut bytes)?;
    if bytes.pop() != Some(b'\n') {
        return Err(invalid(
            "child readiness exceeds the bounded line or ended early",
        ));
    }
    let ready: Value = serde_json::from_slice(&bytes)?;
    if ready["version"] != 1 {
        return Err(invalid("unsupported child readiness version"));
    }
    let instance_id = ready["instance_id"]
        .as_str()
        .filter(|id| is_lower_hex(id, 32))
        .ok_or_else(|| invalid("child readiness has no window identity"))?
        .to_owned();
    let attach_address = ready["consent_address"]
        .as_str()
        .and_then(loopback_address)
        .ok_or_else(|| invalid("child readiness has no loopback attach address"))?;
    let address = ready["live_bridge_address"]
        .as_str()
        .and_then(loopback_address)
        .ok_or_else(|| invalid("child readiness has no loopback bridge address"))?;
    Ok((
        Window {
            registry_path: default_root()
                .ok_or_else(|| invalid("per-user discovery directory unavailable"))?
                .join(format!("{instance_id}.json")),
            instance_id,
            document,
            in_use: true,
            attach_address,
        },
        address,
    ))
}

pub fn attach(window: &Window) -> io::Result<Grant> {
    let entry: Value =
        serde_json::from_slice(&crate::local_auth::read_private(&window.registry_path)?)?;
    if entry["version"] != 1
        || entry["instance_id"] != window.instance_id
        || entry["consent_address"].as_str().and_then(loopback_address)
            != Some(window.attach_address)
    {
        return Err(invalid(
            "private bootstrap does not identify the selected window",
        ));
    }
    let bootstrap = entry["bootstrap"]
        .as_str()
        .filter(|value| is_lower_hex(value, 64))
        .ok_or_else(|| invalid("private registry has no bootstrap credential"))?;
    let reply = exchange(
        window.attach_address,
        "attach",
        &window.instance_id,
        ATTACH_TIMEOUT,
        Some(bootstrap),
    )?;
    if reply["status"] != "allowed" {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the window refused the attach request",
        ));
    }
    let address = reply["live_bridge_address"]
        .as_str()
        .and_then(loopback_address)
        .ok_or_else(|| invalid("attach reply without a loopback bridge address"))?;
    let token = reply["token"]
        .as_str()
        .filter(|token| is_lower_hex(token, 64))
        .ok_or_else(|| invalid("attach reply without a credential"))?
        .to_owned();
    Ok(Grant { address, token })
}

fn registry_entry(path: &Path) -> Option<(String, SocketAddr)> {
    let instance_id = path.file_name()?.to_str()?.strip_suffix(".json")?;
    if !is_lower_hex(instance_id, 32) {
        return None;
    }
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_LINE_BYTES {
        return None;
    }
    let entry: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if entry["version"] != 1 || entry["instance_id"] != instance_id {
        return None;
    }
    let address = loopback_address(entry["consent_address"].as_str()?)?;
    Some((instance_id.to_owned(), address))
}

fn probe(root: &Path, instance_id: &str, attach_address: SocketAddr) -> io::Result<Window> {
    let reply = exchange(attach_address, "list", instance_id, PROBE_TIMEOUT, None)?;
    let document = reply["document"]
        .as_str()
        .ok_or_else(|| invalid("list reply without a document name"))?
        .to_owned();
    Ok(Window {
        registry_path: root.join(format!("{instance_id}.json")),
        instance_id: instance_id.to_owned(),
        document,
        in_use: reply["status"] == "busy",
        attach_address,
    })
}

fn exchange(
    address: SocketAddr,
    action: &str,
    instance_id: &str,
    timeout: Duration,
    bootstrap: Option<&str>,
) -> io::Result<Value> {
    let nonce = nonce()?;
    let mut stream = TcpStream::connect_timeout(&address, PROBE_TIMEOUT)?;
    stream.set_write_timeout(Some(PROBE_TIMEOUT))?;
    stream.set_read_timeout(Some(timeout))?;
    let mut request = json!({"version": 1, "action": action, "nonce": nonce});
    if let Some(bootstrap) = bootstrap {
        request["bootstrap"] = Value::String(bootstrap.to_owned());
    }
    let mut line = request.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut reply = Vec::new();
    BufReader::new(stream.take(MAX_LINE_BYTES)).read_until(b'\n', &mut reply)?;
    if reply.pop() != Some(b'\n') {
        return Err(invalid("attach endpoint reply is not one line"));
    }
    let reply: Value = serde_json::from_slice(&reply)?;
    if reply["version"] != 1 || reply["nonce"] != nonce || reply["instance_id"] != instance_id {
        return Err(invalid(
            "attach endpoint answered for another request or window",
        ));
    }
    Ok(reply)
}

pub(crate) fn nonce() -> io::Result<String> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn loopback_address(text: &str) -> Option<SocketAddr> {
    text.parse::<SocketAddr>()
        .ok()
        .filter(|address| address.ip().is_loopback())
}

fn is_lower_hex(text: &str, length: usize) -> bool {
    text.len() == length && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}
