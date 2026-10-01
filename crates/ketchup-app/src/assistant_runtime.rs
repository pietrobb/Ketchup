use crate::{AssistantTransport, AssistantTransportResponse};
use ketchup_assistant::request_invalid::AssistantRequestInvalid;
use ketchup_assistant::sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantApiDiagnostics, AssistantCapability,
    AssistantDistribution, AssistantHandshake,
};
use ketchup_model::graph::sha256_hex;
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

/// Why the Assistant process could not be launched or verified.
#[derive(Debug)]
pub enum AssistantLaunchError {
    /// A launch input or setting breaks its rule; nothing was started.
    InvalidSetting {
        setting: &'static str,
        requirement: &'static str,
    },
    /// A launch component is missing or unreadable; the I/O error is the cause when one exists.
    Unavailable {
        component: String,
        cause: Option<io::Error>,
    },
    /// A runtime file differs from the identity this build was released with.
    IdentityMismatch {
        component: String,
    },
    /// A runtime file resolves outside the trusted install root.
    OutsideInstallRoot {
        component: String,
    },
    UnsupportedProvider {
        provider: String,
    },
    /// Starting or stopping the Assistant process failed; its error is the cause.
    Failed(Box<dyn std::error::Error + Send + Sync>),
}

impl AssistantLaunchError {
    fn unavailable(component: &str) -> Self {
        Self::Unavailable {
            component: component.to_owned(),
            cause: None,
        }
    }

    fn io(component: &str) -> impl FnOnce(io::Error) -> Self + '_ {
        move |cause| Self::Unavailable {
            component: component.to_owned(),
            cause: Some(cause),
        }
    }

    fn failed(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Failed(cause.into())
    }
}

impl std::fmt::Display for AssistantLaunchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSetting {
                setting,
                requirement,
            } => write!(formatter, "{setting} must be {requirement}"),
            Self::Unavailable {
                component,
                cause: None,
            } => write!(formatter, "{component} is unavailable"),
            Self::Unavailable {
                component,
                cause: Some(cause),
            } => write!(formatter, "{component} is unavailable: {cause}"),
            Self::IdentityMismatch { component } => {
                write!(formatter, "{component} identity mismatch")
            }
            Self::OutsideInstallRoot { component } => write!(
                formatter,
                "{component} escapes the trusted Assistant install root"
            ),
            Self::UnsupportedProvider { provider } => {
                write!(
                    formatter,
                    "unsupported public Assistant provider {provider}"
                )
            }
            Self::Failed(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for AssistantLaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable {
                cause: Some(cause), ..
            } => Some(cause),
            Self::Failed(cause) => cause.source(),
            _ => None,
        }
    }
}

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

impl AssistantTransportResponse {
    /// Checks the reply and its API diagnostics against the request schema;
    /// action payloads are checked when they are planned.
    pub(crate) fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        self.result.validate()?;
        self.diagnostics
            .as_ref()
            .map_or(Ok(()), AssistantApiDiagnostics::validate)
    }
}

impl AssistantTransport for ProcessAssistantTransport {
    fn chat(
        &self,
        handshake: AssistantHandshake,
        request_id: &str,
        message: &str,
        context: &serde_json::Value,
        cancellation: AssistantCancellation,
    ) -> Result<ketchup_assistant::sidecar::AssistantChatResult, String> {
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
                let launch = public_assistant_launch(&handshake.provider)
                    .map_err(|error| error.to_string())?;
                AssistantProcessClient::spawn_isolated_with_cancellation(
                    &launch,
                    handshake,
                    ASSISTANT_TIMEOUT,
                    cancellation,
                )
            }
            AssistantDistribution::PrivateOauth => {
                let launch = private_assistant_launch().map_err(|error| error.to_string())?;
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

fn first_existing_assistant_path(
    candidates: impl IntoIterator<Item = Option<PathBuf>>,
) -> Option<PathBuf> {
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.is_absolute() && path.is_file())
}

#[cfg(windows)]
fn persistent_user_environment_path(name: &str) -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .ok()?
        .get_value::<String, _>(name)
        .ok()
        .map(PathBuf::from)
}

#[cfg(not(windows))]
fn persistent_user_environment_path(_name: &str) -> Option<PathBuf> {
    None
}

pub fn private_assistant_launch() -> Result<AssistantProcessLaunch, AssistantLaunchError> {
    let beside_app = std::env::current_exe().ok().and_then(|path| {
        path.parent()
            .map(|parent| parent.join("KetchupPrivateAssistant.exe"))
    });
    let executable = first_existing_assistant_path([
        std::env::var_os("KETCHUP_PRIVATE_ASSISTANT").map(PathBuf::from),
        beside_app,
        persistent_user_environment_path("KETCHUP_PRIVATE_ASSISTANT"),
    ])
    .ok_or_else(|| {
        AssistantLaunchError::unavailable("KetchupPrivateAssistant.exe in a trusted location")
    })?;
    private_assistant_launch_for_executable(&executable)
}

#[doc(hidden)]
pub fn private_assistant_launch_for_executable(
    executable: &Path,
) -> Result<AssistantProcessLaunch, AssistantLaunchError> {
    if !executable.is_absolute() {
        return Err(AssistantLaunchError::InvalidSetting {
            setting: "private OAuth Assistant executable",
            requirement: "an absolute path",
        });
    }
    let executable = executable
        .canonicalize()
        .map_err(AssistantLaunchError::io("private OAuth Assistant"))?;
    let bytes = read_bounded_regular_file(
        &executable,
        ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES,
    )
    .map_err(AssistantLaunchError::io(
        "private OAuth Assistant as a bounded file",
    ))?;
    let working_directory = executable
        .parent()
        .ok_or_else(|| AssistantLaunchError::unavailable("private OAuth Assistant install root"))?
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

pub fn verify_public_assistant_runtime() -> Result<(), AssistantLaunchError> {
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
    .map_err(AssistantLaunchError::failed)?;
    client.shutdown().map_err(AssistantLaunchError::failed)
}

fn public_assistant_launch(provider: &str) -> Result<AssistantProcessLaunch, AssistantLaunchError> {
    let install_root = public_install_root()?;
    let interpreter = installed_public_python(&install_root)?;
    public_assistant_launch_for_install_root(
        &install_root,
        &interpreter,
        PINNED_PUBLIC_PYTHON_SHA256,
        provider,
    )
}

fn public_install_root() -> Result<PathBuf, AssistantLaunchError> {
    std::env::current_exe()
        .map_err(AssistantLaunchError::io("current executable"))?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AssistantLaunchError::unavailable("application install root"))
}

#[cfg(windows)]
fn installed_public_python(install_root: &Path) -> Result<PathBuf, AssistantLaunchError> {
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
    Err(AssistantLaunchError::unavailable(
        "the pinned installer-managed Python 3.11 runtime",
    ))
}

#[cfg(not(windows))]
fn installed_public_python(install_root: &Path) -> Result<PathBuf, AssistantLaunchError> {
    let candidate = install_root.join("python3");
    let path = candidate.canonicalize().map_err(AssistantLaunchError::io(
        "the co-located pinned Python runtime",
    ))?;
    let bytes = read_bounded_regular_file(
        &path,
        ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES,
    )
    .map_err(AssistantLaunchError::io(
        "the co-located pinned Python runtime",
    ))?;
    if sha256_hex(&bytes) != PINNED_PUBLIC_PYTHON_SHA256 {
        return Err(AssistantLaunchError::IdentityMismatch {
            component: "the co-located Python runtime".to_owned(),
        });
    }
    Ok(path)
}

#[doc(hidden)]
pub fn public_assistant_launch_for_install_root(
    install_root: &Path,
    interpreter: &Path,
    interpreter_sha256: &str,
    provider: &str,
) -> Result<AssistantProcessLaunch, AssistantLaunchError> {
    if !interpreter.is_absolute() {
        return Err(AssistantLaunchError::InvalidSetting {
            setting: "public Assistant interpreter",
            requirement: "an absolute path",
        });
    }
    if interpreter_sha256.len() != 64
        || !interpreter_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(AssistantLaunchError::InvalidSetting {
            setting: "public Assistant interpreter identity",
            requirement: "a lowercase SHA-256",
        });
    }
    let executable = interpreter
        .canonicalize()
        .map_err(AssistantLaunchError::io("public Assistant interpreter"))?;
    if !executable.is_file() {
        return Err(AssistantLaunchError::InvalidSetting {
            setting: "public Assistant interpreter",
            requirement: "a regular file",
        });
    }

    let root = install_root
        .canonicalize()
        .map_err(AssistantLaunchError::io("public Assistant install root"))?;
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
) -> Result<PublicAssistantEnvironment, AssistantLaunchError> {
    let api_key = match provider {
        "anthropic-api" => "ANTHROPIC_API_KEY",
        "openai-api" => "OPENAI_API_KEY",
        _ => {
            return Err(AssistantLaunchError::UnsupportedProvider {
                provider: provider.to_owned(),
            });
        }
    };
    let invalid = |setting, requirement| AssistantLaunchError::InvalidSetting {
        setting,
        requirement,
    };
    let mut environment = Vec::new();
    let mut environment_files = Vec::new();
    if let Some(value) = source(api_key).filter(|value| !value.is_empty()) {
        environment.push((OsString::from(api_key), value));
    }
    if let Some(value) = source("KETCHUP_ASSISTANT_HTTPS_PROXY").filter(|value| !value.is_empty()) {
        let text = value
            .to_str()
            .ok_or_else(|| invalid("Assistant HTTPS proxy", "UTF-8 text"))?;
        let endpoint = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"))
            .ok_or_else(|| invalid("Assistant HTTPS proxy", "an http:// or https:// URL"))?;
        if endpoint.is_empty()
            || text.len() > 2_048
            || text.contains('@')
            || text.chars().any(char::is_whitespace)
        {
            return Err(invalid(
                "Assistant HTTPS proxy",
                "a bounded URL without credentials or whitespace",
            ));
        }
        environment.push((OsString::from("HTTPS_PROXY"), value));
    }
    if let Some(value) = source("KETCHUP_ASSISTANT_NO_PROXY").filter(|value| !value.is_empty()) {
        let text = value
            .to_str()
            .ok_or_else(|| invalid("Assistant NO_PROXY", "UTF-8 text"))?;
        if text.len() > 2_048 || text.chars().any(|character| character.is_control()) {
            return Err(invalid(
                "Assistant NO_PROXY",
                "at most 2048 bytes without control characters",
            ));
        }
        environment.push((OsString::from("NO_PROXY"), value));
    }
    if let Some(value) = source("KETCHUP_ASSISTANT_CA_BUNDLE").filter(|value| !value.is_empty()) {
        let configured = PathBuf::from(value);
        if !configured.is_absolute() {
            return Err(invalid("Assistant CA bundle", "an absolute file path"));
        }
        let canonical = configured
            .canonicalize()
            .map_err(AssistantLaunchError::io("Assistant CA bundle"))?;
        let bytes = read_bounded_regular_file(&canonical, MAX_ASSISTANT_CA_BUNDLE_BYTES).map_err(
            AssistantLaunchError::io("Assistant CA bundle as a bounded file"),
        )?;
        let mut snapshot = tempfile::Builder::new()
            .prefix("ketchup-assistant-ca-")
            .suffix(".pem")
            .tempfile()
            .map_err(AssistantLaunchError::io("Assistant CA bundle snapshot"))?;
        snapshot
            .write_all(&bytes)
            .and_then(|()| snapshot.flush())
            .map_err(AssistantLaunchError::io("Assistant CA bundle snapshot"))?;
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

fn verified_runtime_file(
    root: &Path,
    relative: &Path,
    expected: &[u8],
) -> Result<PathBuf, AssistantLaunchError> {
    let component = relative.display().to_string();
    let path = root.join(relative);
    let canonical = path
        .canonicalize()
        .map_err(AssistantLaunchError::io(&component))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(AssistantLaunchError::OutsideInstallRoot { component });
    }
    let actual = read_bounded_regular_file(&canonical, expected.len() as u64)
        .map_err(AssistantLaunchError::io(&component))?;
    if actual != expected {
        return Err(AssistantLaunchError::IdentityMismatch { component });
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{
        first_existing_assistant_path, public_assistant_environment, read_bounded_regular_file,
    };
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    #[test]
    fn private_assistant_discovery_skips_invalid_candidates_in_precedence_order() {
        let directory = tempfile::tempdir().unwrap();
        let process = directory.path().join("process.exe");
        let beside = directory.path().join("beside.exe");
        let persistent = directory.path().join("persistent.exe");
        for path in [&process, &beside, &persistent] {
            std::fs::write(path, b"sidecar").unwrap();
        }

        assert_eq!(
            first_existing_assistant_path([
                Some(process.clone()),
                Some(beside.clone()),
                Some(persistent.clone()),
            ]),
            Some(process.clone())
        );
        std::fs::remove_file(&process).unwrap();
        assert_eq!(
            first_existing_assistant_path([
                Some(process),
                Some(beside.clone()),
                Some(persistent.clone()),
            ]),
            Some(beside.clone())
        );
        std::fs::remove_file(&beside).unwrap();
        assert_eq!(
            first_existing_assistant_path([
                Some(std::path::PathBuf::from("relative.exe")),
                Some(beside),
                Some(persistent.clone()),
            ]),
            Some(persistent)
        );
    }

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
            assert!(matches!(
                error,
                super::AssistantLaunchError::InvalidSetting {
                    setting: "Assistant HTTPS proxy",
                    ..
                }
            ));
        }
        let error = public_assistant_environment("openai-api", |name| {
            (name == "KETCHUP_ASSISTANT_CA_BUNDLE")
                .then(|| OsString::from("relative-company-ca.pem"))
        })
        .unwrap_err();
        assert!(matches!(
            error,
            super::AssistantLaunchError::InvalidSetting {
                setting: "Assistant CA bundle",
                requirement: "an absolute file path",
            }
        ));
    }
}
