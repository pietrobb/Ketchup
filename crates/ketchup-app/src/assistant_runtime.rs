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
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const ASSISTANT_TIMEOUT: Duration = Duration::from_secs(300);
const PINNED_PUBLIC_PYTHON_SHA256: &str =
    "5f7b89a612c9b8af1d6456cdfcd1dbe5ca630849e79aebced9bee9a6694952ec";
const PUBLIC_ASSISTANT: &[u8] = include_bytes!("../../../sdk/python/ketchup_assistant.py");
const PUBLIC_ASSISTANT_PROTOCOL: &[u8] =
    include_bytes!("../../../sdk/python/ketchup_assistant_protocol.py");
const PUBLIC_ASSISTANT_BOOTSTRAP: &str = r#"import hashlib,pathlib,sys,types

def checked_source(path, expected):
    source = pathlib.Path(path).read_bytes()
    if hashlib.sha256(source).hexdigest() != expected:
        raise SystemExit("public Assistant runtime identity mismatch")
    return source

script_path, script_hash, protocol_path, protocol_hash = sys.argv[1:5]
protocol = types.ModuleType("ketchup_assistant_protocol")
protocol.__file__ = protocol_path
protocol.__package__ = None
sys.modules[protocol.__name__] = protocol
exec(compile(checked_source(protocol_path, protocol_hash), protocol_path, "exec"), protocol.__dict__)
namespace = {"__name__": "__main__", "__file__": script_path, "__package__": None}
exec(compile(checked_source(script_path, script_hash), script_path, "exec"), namespace)
"#;

pub(crate) struct ProcessAssistantTransport;

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
    let file = fs::File::open(&executable)
        .map_err(|error| format!("private OAuth Assistant is unreadable: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("private OAuth Assistant identity is unavailable: {error}"))?;
    if !metadata.is_file()
        || metadata.len() > ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES
    {
        return Err("private OAuth Assistant is not a bounded file".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("private OAuth Assistant is unreadable: {error}"))?;
    if bytes.len() as u64 > ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES {
        return Err("private OAuth Assistant is not a bounded file".to_owned());
    }
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
        let Ok(file) = fs::File::open(&path) else {
            continue;
        };
        let Ok(metadata) = file.metadata() else {
            continue;
        };
        if !metadata.is_file()
            || metadata.len() > ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES
        {
            continue;
        }
        let mut bytes = Vec::new();
        if file
            .take(ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_ok()
            && bytes.len() as u64 <= ketchup_scheduler::assistant::MAX_ASSISTANT_EXECUTABLE_BYTES
            && sha256_hex(&bytes) == PINNED_PUBLIC_PYTHON_SHA256
        {
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
    let bytes = fs::read(&path)
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
            protocol.into_os_string(),
            OsString::from(sha256_hex(PUBLIC_ASSISTANT_PROTOCOL)),
        ],
        working_directory,
        environment,
    })
}

fn public_assistant_environment(
    provider: &str,
    source: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<(OsString, OsString)>, String> {
    let api_key = match provider {
        "anthropic-api" => "ANTHROPIC_API_KEY",
        "openai-api" => "OPENAI_API_KEY",
        _ => return Err("unsupported public Assistant provider".to_owned()),
    };
    let mut environment = Vec::new();
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
        let metadata = canonical
            .metadata()
            .map_err(|error| format!("Assistant CA bundle identity is unavailable: {error}"))?;
        if !metadata.is_file() || metadata.len() > 4 * 1024 * 1024 {
            return Err("Assistant CA bundle must be a bounded file".to_owned());
        }
        environment.push((OsString::from("SSL_CERT_FILE"), canonical.into_os_string()));
    }
    #[cfg(windows)]
    if let Some(value) = source("SYSTEMROOT").filter(|value| !value.is_empty()) {
        environment.push((OsString::from("SYSTEMROOT"), value));
    }
    Ok(environment)
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
    let metadata = canonical
        .metadata()
        .map_err(|error| format!("{} identity is unavailable: {error}", relative.display()))?;
    if metadata.len() != expected.len() as u64 {
        return Err(format!("{} identity mismatch", relative.display()));
    }
    let actual = fs::read(&canonical)
        .map_err(|error| format!("{} identity is unreadable: {error}", relative.display()))?;
    if actual != expected {
        return Err(format!("{} identity mismatch", relative.display()));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::public_assistant_environment;
    use std::collections::BTreeMap;
    use std::ffi::OsString;

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

        assert!(environment.contains(&(
            OsString::from("HTTPS_PROXY"),
            OsString::from("http://proxy.example:8080")
        )));
        assert!(environment.contains(&(
            OsString::from("NO_PROXY"),
            OsString::from("localhost,.example.test")
        )));
        assert!(environment.iter().any(|(name, value)| {
            name == "SSL_CERT_FILE" && std::path::PathBuf::from(value).is_absolute()
        }));
        assert!(environment.iter().all(|(name, value)| {
            name != "PATH" && value != "http://ambient.invalid" && value != "ambient-invalid.pem"
        }));
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
