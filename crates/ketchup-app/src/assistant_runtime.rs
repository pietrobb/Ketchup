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
    let interpreter = std::env::var_os("KETCHUP_PYTHON")
        .map(PathBuf::from)
        .ok_or_else(|| "KETCHUP_PYTHON must name an absolute Python interpreter".to_owned())?;
    let interpreter_sha256 = std::env::var("KETCHUP_PYTHON_SHA256")
        .map_err(|_| "KETCHUP_PYTHON_SHA256 must pin the Python interpreter".to_owned())?;
    public_assistant_launch_for_install_root(
        &install_root,
        &interpreter,
        &interpreter_sha256,
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

    let mut environment = Vec::new();
    let api_key = match provider {
        "anthropic-api" => "ANTHROPIC_API_KEY",
        "openai-api" => "OPENAI_API_KEY",
        _ => return Err("unsupported public Assistant provider".to_owned()),
    };
    if let Some(value) = std::env::var_os(api_key).filter(|value| !value.is_empty()) {
        environment.push((OsString::from(api_key), value));
    }
    #[cfg(windows)]
    if let Some(value) = std::env::var_os("SYSTEMROOT").filter(|value| !value.is_empty()) {
        environment.push((OsString::from("SYSTEMROOT"), value));
    }

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
