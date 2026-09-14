use crate::{AssistantTransport, AssistantTransportResponse};
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantCapability, AssistantDistribution, AssistantHandshake,
};
use ketchup_core::graph::sha256_hex;
use ketchup_scheduler::assistant::{
    AssistantCancellation, AssistantProcessClient, AssistantProcessLaunch,
};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const ASSISTANT_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_ASSISTANT_CA_BUNDLE_BYTES: u64 = 4 * 1024 * 1024;
const PINNED_PUBLIC_PYTHON_SHA256: &str =
    "5f7b89a612c9b8af1d6456cdfcd1dbe5ca630849e79aebced9bee9a6694952ec";
const PUBLIC_ASSISTANT: &[u8] = include_bytes!("../../../sdk/python/ketchup_assistant.py");
const PUBLIC_ASSISTANT_PROTOCOL: &[u8] =
    include_bytes!("../../../sdk/python/ketchup_assistant_protocol.py");
const PUBLIC_ASSISTANT_BOOTSTRAP: &str = r#"import hashlib,pathlib,sys,types

def checked_source(path, expected, expected_size):
    expected_size = int(expected_size)
    with pathlib.Path(path).open("rb") as source_file:
        source = source_file.read(expected_size + 1)
    if len(source) > expected_size:
        raise SystemExit("public Assistant runtime source exceeds its bounded identity")
    if len(source) != expected_size or hashlib.sha256(source).hexdigest() != expected:
        raise SystemExit("public Assistant runtime identity mismatch")
    return source

script_path, script_hash, script_size, protocol_path, protocol_hash, protocol_size = sys.argv[1:7]
protocol = types.ModuleType("ketchup_assistant_protocol")
protocol.__file__ = protocol_path
protocol.__package__ = None
sys.modules[protocol.__name__] = protocol
exec(compile(checked_source(protocol_path, protocol_hash, protocol_size), protocol_path, "exec"), protocol.__dict__)
namespace = {"__name__": "__main__", "__file__": script_path, "__package__": None}
exec(compile(checked_source(script_path, script_hash, script_size), script_path, "exec"), namespace)
"#;

pub(crate) struct ProcessAssistantTransport;

#[derive(Debug)]
struct PublicAssistantEnvironment {
    variables: Vec<(OsString, OsString)>,
    files: Vec<Arc<tempfile::NamedTempFile>>,
}

fn read_bounded_regular_file(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds its bounded identity",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file grew beyond its bounded identity",
        ));
    }
    Ok(bytes)
}

impl AssistantTransport for ProcessAssistantTransport {
    fn chat(
        &self,
        handshake: AssistantHandshake,
        request_id: &str,
        message: &str,
        context: &serde_json::Value,
        cancellation: AssistantCancellation,
    ) -> Result<ketchup_core::assistant_sidecar::AssistantChatResult, String> {
        self.chat_with_diagnostics(handshake, request_id, message, context, cancellation)
            .map(|response| response.result)
    }

    fn chat_with_diagnostics(
        &self,
        handshake: AssistantHandshake,
        request_id: &str,
        message: &str,
        context: &serde_json::Value,
        cancellation: AssistantCancellation,
    ) -> Result<AssistantTransportResponse, String> {
        let mut client = match handshake.distribution {
            AssistantDistribution::PublicApi => {
                let launch = public_assistant_launch(&handshake.provider)?;
                AssistantProcessClient::spawn_isolated_with_cancellation(
                    &launch,
                    handshake,
                    ASSISTANT_TIMEOUT,
                    cancellation,
                )
            }
            AssistantDistribution::PrivateOauth => {
                let launch = private_assistant_launch()?;
                AssistantProcessClient::spawn_isolated_with_cancellation(
                    &launch,
                    handshake,
                    ASSISTANT_TIMEOUT,
                    cancellation,
                )
            }
        }
        .map_err(|error| error.to_string())?;
        let answer = client
            .chat_exchange(request_id, message, context)
            .map(|exchange| AssistantTransportResponse {
                result: exchange.result,
                cad_edit_program: exchange.cad_edit_program,
                fea_review: exchange.fea_review,
                diagnostics: exchange.diagnostics,
            })
            .map_err(|error| error.to_string());
        let _ = client.shutdown();
        answer
    }
}

pub fn private_assistant_launch() -> Result<AssistantProcessLaunch, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("current executable is unavailable: {error}"))?
        .parent()
        .map(|parent| parent.join("KetchupPrivateAssistant.exe"))
        .ok_or_else(|| "application install root is unavailable".to_owned())?;
    if !executable.is_file() {
        return Err("KetchupPrivateAssistant.exe was not found beside the application".to_owned());
    }
    private_assistant_launch_for_executable(&executable)
}

#[doc(hidden)]
pub fn private_assistant_launch_for_executable(
    executable: &Path,
) -> Result<AssistantProcessLaunch, String> {
    if !executable.is_absolute() {
        return Err("private OAuth Assistant executable must be absolute".to_owned());
    }
    let executable = executable
        .canonicalize()
        .map_err(|error| format!("private OAuth Assistant is unavailable: {error}"))?;
    let bytes = read_bounded_regular_file(
        &executable,
        ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES,
    )
    .map_err(|error| format!("private OAuth Assistant is not a bounded file: {error}"))?;
    let working_directory = executable
        .parent()
        .ok_or_else(|| "private OAuth Assistant install root is unavailable".to_owned())?
        .to_path_buf();
    let environment = [
        "SYSTEMROOT",
        "WINDIR",
        "USERPROFILE",
        "HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
    ]
    .into_iter()
    .filter_map(|name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(|value| (OsString::from(name), value))
    })
    .collect();
    Ok(AssistantProcessLaunch {
        executable_sha256: sha256_hex(&bytes),
        executable,
        arguments: Vec::new(),
        working_directory,
        environment,
        environment_files: Vec::new(),
    })
}

pub fn verify_public_assistant_runtime() -> Result<(), String> {
    let handshake = AssistantHandshake {
        protocol_version: ASSISTANT_PROTOCOL_VERSION,
        distribution: AssistantDistribution::PublicApi,
        provider: "anthropic-api".to_owned(),
        model: "claude-sonnet-4-6".to_owned(),
        capabilities: BTreeSet::from([
            AssistantCapability::Chat,
            AssistantCapability::LocalMemory,
            AssistantCapability::QueryDocument,
            AssistantCapability::ProposeWorkflowIntent,
        ]),
    };
    let launch = public_assistant_launch(&handshake.provider)?;
    let mut client = AssistantProcessClient::spawn_isolated_with_cancellation(
        &launch,
        handshake,
        Duration::from_secs(30),
        AssistantCancellation::default(),
    )
    .map_err(|error| error.to_string())?;
    client.shutdown().map_err(|error| error.to_string())
}

fn public_assistant_launch(provider: &str) -> Result<AssistantProcessLaunch, String> {
    let install_root = public_install_root()?;
    let interpreter = installed_public_python(&install_root)?;
    public_assistant_launch_for_install_root(
        &install_root,
        &interpreter,
        PINNED_PUBLIC_PYTHON_SHA256,
        provider,
    )
}

fn public_install_root() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| format!("current executable is unavailable: {error}"))?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "application install root is unavailable".to_owned())
}

#[cfg(windows)]
fn installed_public_python(install_root: &Path) -> Result<PathBuf, String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

    let mut candidates = vec![install_root.join("python.exe")];
    if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(
        r"Software\Python\PythonCore\3.11\InstallPath",
        KEY_READ | KEY_WOW64_64KEY,
    ) {
        if let Ok(path) = key.get_value::<OsString, _>("ExecutablePath") {
            candidates.push(PathBuf::from(path));
        } else if let Ok(path) = key.get_value::<OsString, _>("") {
            candidates.push(PathBuf::from(path).join("python.exe"));
        }
    }
    for candidate in candidates {
        let Ok(path) = candidate.canonicalize() else {
            continue;
        };
        let Ok(bytes) = read_bounded_regular_file(
            &path,
            ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES,
        ) else {
            continue;
        };
        if sha256_hex(&bytes) == PINNED_PUBLIC_PYTHON_SHA256 {
            return Ok(path);
        }
    }
    Err("the pinned installer-managed Python 3.11 runtime is unavailable".to_owned())
}

#[cfg(not(windows))]
fn installed_public_python(install_root: &Path) -> Result<PathBuf, String> {
    let candidate = install_root.join("python3");
    let path = candidate
        .canonicalize()
        .map_err(|_| "the co-located pinned Python runtime is unavailable".to_owned())?;
    let bytes = read_bounded_regular_file(
        &path,
        ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES,
    )
    .map_err(|_| "the co-located pinned Python runtime is unavailable".to_owned())?;
    if sha256_hex(&bytes) != PINNED_PUBLIC_PYTHON_SHA256 {
        return Err(
            "the co-located Python runtime identity does not match the release pin".to_owned(),
        );
    }
    Ok(path)
}

#[doc(hidden)]
pub fn public_assistant_launch_for_install_root(
    install_root: &Path,
    interpreter: &Path,
    interpreter_sha256: &str,
    provider: &str,
) -> Result<AssistantProcessLaunch, String> {
    if !interpreter.is_absolute() {
        return Err("public Assistant interpreter must be absolute".to_owned());
    }
    if interpreter_sha256.len() != 64
        || !interpreter_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("public Assistant interpreter identity must be lowercase SHA-256".to_owned());
    }
    let executable = interpreter
        .canonicalize()
        .map_err(|error| format!("public Assistant interpreter is unavailable: {error}"))?;
    if !executable.is_file() {
        return Err("public Assistant interpreter is not a file".to_owned());
    }

    let root = install_root
        .canonicalize()
        .map_err(|error| format!("public Assistant install root is unavailable: {error}"))?;
    let script = verified_runtime_file(&root, Path::new("ketchup_assistant.py"), PUBLIC_ASSISTANT)?;
    let protocol = verified_runtime_file(
        &root,
        Path::new("ketchup_assistant_protocol.py"),
        PUBLIC_ASSISTANT_PROTOCOL,
    )?;
    let working_directory = script
        .parent()
        .expect("verified public Assistant script has a parent")
        .to_path_buf();

    let environment = public_assistant_environment(provider, |name| std::env::var_os(name))?;

    Ok(AssistantProcessLaunch {
        executable,
        executable_sha256: interpreter_sha256.to_owned(),
        arguments: vec![
            OsString::from("-I"),
            OsString::from("-B"),
            OsString::from("-c"),
            OsString::from(PUBLIC_ASSISTANT_BOOTSTRAP),
            script.into_os_string(),
            OsString::from(sha256_hex(PUBLIC_ASSISTANT)),
            OsString::from(PUBLIC_ASSISTANT.len().to_string()),
            protocol.into_os_string(),
            OsString::from(sha256_hex(PUBLIC_ASSISTANT_PROTOCOL)),
            OsString::from(PUBLIC_ASSISTANT_PROTOCOL.len().to_string()),
        ],
        working_directory,
        environment: environment.variables,
        environment_files: environment.files,
    })
}

fn public_assistant_environment(
    provider: &str,
    source: impl Fn(&str) -> Option<OsString>,
) -> Result<PublicAssistantEnvironment, String> {
    let api_key = match provider {
        "anthropic-api" => "ANTHROPIC_API_KEY",
        "openai-api" => "OPENAI_API_KEY",
        _ => return Err("unsupported public Assistant provider".to_owned()),
    };
    let mut environment = Vec::new();
    let mut environment_files = Vec::new();
    if let Some(value) = source(api_key).filter(|value| !value.is_empty()) {
        environment.push((OsString::from(api_key), value));
    }
    if let Some(value) = source("KETCHUP_ASSISTANT_HTTPS_PROXY").filter(|value| !value.is_empty()) {
        let text = value
            .to_str()
            .ok_or_else(|| "Assistant HTTPS proxy must be UTF-8".to_owned())?;
        let endpoint = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"))
            .ok_or_else(|| "Assistant HTTPS proxy must use http:// or https://".to_owned())?;
        if endpoint.is_empty()
            || text.len() > 2_048
            || text.contains('@')
            || text.chars().any(char::is_whitespace)
        {
            return Err("Assistant HTTPS proxy is invalid or contains credentials".to_owned());
        }
        environment.push((OsString::from("HTTPS_PROXY"), value));
    }
    if let Some(value) = source("KETCHUP_ASSISTANT_NO_PROXY").filter(|value| !value.is_empty()) {
        let text = value
            .to_str()
            .ok_or_else(|| "Assistant NO_PROXY must be UTF-8".to_owned())?;
        if text.len() > 2_048 || text.chars().any(|character| character.is_control()) {
            return Err("Assistant NO_PROXY is invalid".to_owned());
        }
        environment.push((OsString::from("NO_PROXY"), value));
    }
    if let Some(value) = source("KETCHUP_ASSISTANT_CA_BUNDLE").filter(|value| !value.is_empty()) {
        let configured = PathBuf::from(value);
        if !configured.is_absolute() {
            return Err("Assistant CA bundle must be an absolute file".to_owned());
        }
        let canonical = configured
            .canonicalize()
            .map_err(|error| format!("Assistant CA bundle is unavailable: {error}"))?;
        let bytes = read_bounded_regular_file(&canonical, MAX_ASSISTANT_CA_BUNDLE_BYTES)
            .map_err(|error| format!("Assistant CA bundle must be a bounded file: {error}"))?;
        let mut snapshot = tempfile::Builder::new()
            .prefix("ketchup-assistant-ca-")
            .suffix(".pem")
            .tempfile()
            .map_err(|error| format!("Assistant CA bundle snapshot is unavailable: {error}"))?;
        snapshot
            .write_all(&bytes)
            .and_then(|()| snapshot.flush())
            .map_err(|error| format!("Assistant CA bundle snapshot failed: {error}"))?;
        let snapshot = Arc::new(snapshot);
        environment.push((
            OsString::from("SSL_CERT_FILE"),
            snapshot.path().as_os_str().to_owned(),
        ));
        environment_files.push(snapshot);
    }
    #[cfg(windows)]
    if let Some(value) = source("SYSTEMROOT").filter(|value| !value.is_empty()) {
        environment.push((OsString::from("SYSTEMROOT"), value));
    }
    Ok(PublicAssistantEnvironment {
        variables: environment,
        files: environment_files,
    })
}

fn verified_runtime_file(root: &Path, relative: &Path, expected: &[u8]) -> Result<PathBuf, String> {
    let path = root.join(relative);
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{} is unavailable: {error}", relative.display()))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(format!(
            "{} escapes the trusted Assistant install root",
            relative.display()
        ));
    }
    let actual = read_bounded_regular_file(&canonical, expected.len() as u64)
        .map_err(|error| format!("{} identity is unreadable: {error}", relative.display()))?;
    if actual != expected {
        return Err(format!("{} identity mismatch", relative.display()));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{public_assistant_environment, read_bounded_regular_file};
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    #[test]
    fn runtime_identity_reader_rejects_oversized_regular_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("runtime.bin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(1024 * 1024).unwrap();

        let error = read_bounded_regular_file(&path, 64).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn public_environment_maps_only_explicit_bounded_network_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let ca_bundle = directory.path().join("company-ca.pem");
        std::fs::write(&ca_bundle, "test certificate bundle").unwrap();
        let values = BTreeMap::from([
            ("ANTHROPIC_API_KEY", OsString::from("secret")),
            (
                "KETCHUP_ASSISTANT_HTTPS_PROXY",
                OsString::from("http://proxy.example:8080"),
            ),
            (
                "KETCHUP_ASSISTANT_NO_PROXY",
                OsString::from("localhost,.example.test"),
            ),
            (
                "KETCHUP_ASSISTANT_CA_BUNDLE",
                ca_bundle.as_os_str().to_owned(),
            ),
            ("HTTPS_PROXY", OsString::from("http://ambient.invalid")),
            ("SSL_CERT_FILE", OsString::from("ambient-invalid.pem")),
            ("PATH", OsString::from("attacker-path")),
        ]);
        let environment =
            public_assistant_environment("anthropic-api", |name| values.get(name).cloned())
                .unwrap();
        assert_eq!(environment.files.len(), 1);

        assert!(environment.variables.contains(&(
            OsString::from("HTTPS_PROXY"),
            OsString::from("http://proxy.example:8080")
        )));
        assert!(environment.variables.contains(&(
            OsString::from("NO_PROXY"),
            OsString::from("localhost,.example.test")
        )));
        assert!(environment.variables.iter().any(|(name, value)| {
            name == "SSL_CERT_FILE" && std::path::PathBuf::from(value).is_absolute()
        }));
        assert!(environment.variables.iter().all(|(name, value)| {
            name != "PATH" && value != "http://ambient.invalid" && value != "ambient-invalid.pem"
        }));
    }

    #[test]
    fn public_environment_snapshots_ca_bundle_before_launch() {
        let directory = tempfile::tempdir().unwrap();
        let ca_bundle = directory.path().join("company-ca.pem");
        std::fs::write(&ca_bundle, b"trusted certificate bundle").unwrap();
        let environment = public_assistant_environment("openai-api", |name| {
            (name == "KETCHUP_ASSISTANT_CA_BUNDLE").then(|| ca_bundle.as_os_str().to_owned())
        })
        .unwrap();
        assert_eq!(environment.files.len(), 1);
        let snapshot = environment
            .variables
            .iter()
            .find_map(|(name, value)| (name == "SSL_CERT_FILE").then(|| value.clone()))
            .map(std::path::PathBuf::from)
            .unwrap();

        std::fs::OpenOptions::new()
            .write(true)
            .open(&ca_bundle)
            .unwrap()
            .set_len(8 * 1024 * 1024)
            .unwrap();

        assert_ne!(snapshot, ca_bundle.canonicalize().unwrap());
        assert_eq!(
            std::fs::read(&snapshot).unwrap(),
            b"trusted certificate bundle"
        );
        drop(environment.files);
        assert!(!snapshot.exists());
    }

    #[test]
    fn public_environment_rejects_unsafe_proxy_and_ca_configuration() {
        for proxy in [
            "socks5://proxy.example:1080",
            "http://user:secret@proxy.example",
            "https://",
            "https://proxy.example/ bad",
        ] {
            let error = public_assistant_environment("openai-api", |name| {
                (name == "KETCHUP_ASSISTANT_HTTPS_PROXY").then(|| OsString::from(proxy))
            })
            .unwrap_err();
            assert!(error.contains("proxy"));
        }
        let error = public_assistant_environment("openai-api", |name| {
            (name == "KETCHUP_ASSISTANT_CA_BUNDLE")
                .then(|| OsString::from("relative-company-ca.pem"))
        })
        .unwrap_err();
        assert!(error.contains("absolute"));
    }
}
