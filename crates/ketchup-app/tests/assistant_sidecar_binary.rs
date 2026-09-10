//! Handshake coverage against the sidecars the shipped app actually launches.
//!
//! The headless assistant suites talk to purpose-written mock sidecars, so they
//! stayed green while the real private OAuth build silently fell behind protocol
//! version 3 and rejected every request in the UI with "unsupported protocol
//! version". These tests spawn the same executables the product resolves at
//! runtime and require a completed handshake.

use ketchup_app::{
    private_assistant_launch, private_assistant_launch_for_executable,
    public_assistant_launch_for_install_root,
};
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantCapability, AssistantDistribution, AssistantHandshake,
};
use ketchup_scheduler::assistant::{AssistantProcessClient, AssistantProcessLaunch};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

fn handshake(
    distribution: AssistantDistribution,
    provider: &str,
    model: &str,
) -> AssistantHandshake {
    AssistantHandshake {
        protocol_version: ASSISTANT_PROTOCOL_VERSION,
        distribution,
        provider: provider.to_owned(),
        model: model.to_owned(),
        capabilities: BTreeSet::from([
            AssistantCapability::Chat,
            AssistantCapability::LocalMemory,
            AssistantCapability::QueryDocument,
            AssistantCapability::ProposeWorkflowIntent,
        ]),
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn public_runtime_root() -> PathBuf {
    repository_root().join("sdk/python")
}

fn absolute_python() -> PathBuf {
    if let Some(configured) = std::env::var_os("KETCHUP_PYTHON").map(PathBuf::from)
        && configured.is_absolute()
    {
        return configured
            .canonicalize()
            .expect("configured Python interpreter");
    }
    let executable = if cfg!(windows) {
        "python.exe"
    } else {
        "python3"
    };
    let output = Command::new(executable)
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .expect("locate Python interpreter");
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .canonicalize()
        .expect("absolute Python interpreter")
}

fn python_sha256() -> String {
    ketchup_core::graph::sha256_hex(&fs::read(absolute_python()).unwrap())
}

fn complete_isolated_handshake(launch: &AssistantProcessLaunch, handshake: AssistantHandshake) {
    let cancellation = ketchup_scheduler::assistant::AssistantCancellation::default();
    let mut client = AssistantProcessClient::spawn_isolated_with_cancellation(
        launch,
        handshake,
        HANDSHAKE_TIMEOUT,
        cancellation,
    )
    .unwrap_or_else(|error| {
        panic!(
            "{} rejected the isolated production handshake: {error}",
            launch.executable.display()
        )
    });
    client.shutdown().expect("sidecar shutdown");
}

#[test]
fn public_sidecar_script_completes_the_production_handshake() {
    let launch = public_assistant_launch_for_install_root(
        &public_runtime_root(),
        &absolute_python(),
        &python_sha256(),
        "anthropic-api",
    )
    .expect("trusted public Assistant runtime");
    complete_isolated_handshake(
        &launch,
        handshake(
            AssistantDistribution::PublicApi,
            "anthropic-api",
            "claude-sonnet-4-6",
        ),
    );
}

#[test]
fn public_sidecar_launch_ignores_attacker_cwd_path_and_python_environment() {
    let launch = public_assistant_launch_for_install_root(
        &public_runtime_root(),
        &absolute_python(),
        &python_sha256(),
        "openai-api",
    )
    .expect("trusted public Assistant runtime");
    let script = public_runtime_root()
        .join("ketchup_assistant.py")
        .canonicalize()
        .unwrap();

    assert_eq!(launch.arguments[0], "-I");
    assert_eq!(launch.arguments[1], "-B");
    assert_eq!(launch.arguments[2], "-c");
    assert_eq!(PathBuf::from(&launch.arguments[4]), script);
    assert_eq!(launch.executable_sha256, python_sha256());
    assert!(launch.executable.is_absolute());
    assert_eq!(launch.working_directory, script.parent().unwrap());
    for forbidden in [
        "PATH",
        "PYTHONPATH",
        "PYTHONHOME",
        "KETCHUP_PYTHON",
        "KETCHUP_PYTHON_SHA256",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "SSL_CERT_FILE",
        "KETCHUP_PUBLIC_ASSISTANT",
    ] {
        assert!(
            launch.environment.iter().all(|(name, _)| name != forbidden),
            "ambient {forbidden} leaked into the public sidecar"
        );
    }
}

#[test]
fn public_sidecar_launch_rejects_relative_interpreter_and_changed_runtime() {
    let relative = public_assistant_launch_for_install_root(
        &public_runtime_root(),
        &PathBuf::from("python.exe"),
        &python_sha256(),
        "anthropic-api",
    )
    .unwrap_err();
    assert!(relative.contains("absolute"));

    let temp = TempDir::new().unwrap();
    let runtime = temp.path();
    fs::copy(
        repository_root().join("sdk/python/ketchup_assistant.py"),
        runtime.join("ketchup_assistant.py"),
    )
    .unwrap();
    fs::write(
        runtime.join("ketchup_assistant_protocol.py"),
        b"changed runtime",
    )
    .unwrap();
    let changed = public_assistant_launch_for_install_root(
        temp.path(),
        &absolute_python(),
        &python_sha256(),
        "anthropic-api",
    )
    .unwrap_err();
    assert!(changed.contains("identity mismatch"));
}

#[test]
fn private_oauth_sidecar_binary_completes_the_production_handshake() {
    let Ok(launch) = private_assistant_launch() else {
        eprintln!("skipped: no private OAuth sidecar is configured on this machine");
        return;
    };
    for forbidden in [
        "PATH",
        "PYTHONPATH",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "KETCHUP_PRIVATE_ASSISTANT",
    ] {
        assert!(
            launch.environment.iter().all(|(name, _)| name != forbidden),
            "ambient {forbidden} leaked into the private OAuth sidecar"
        );
    }
    assert!(launch.executable.is_absolute());
    assert_eq!(
        launch.working_directory,
        launch.executable.parent().unwrap()
    );
    complete_isolated_handshake(
        &launch,
        handshake(
            AssistantDistribution::PrivateOauth,
            "codex-oauth",
            "gpt-5.6-sol",
        ),
    );
}

#[test]
fn private_oauth_launch_rejects_relative_executables_and_pins_identity() {
    let relative =
        private_assistant_launch_for_executable(&PathBuf::from("KetchupPrivateAssistant.exe"))
            .unwrap_err();
    assert!(relative.contains("absolute"));

    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let launch = private_assistant_launch_for_executable(&executable).unwrap();
    assert_eq!(launch.executable, executable);
    assert_eq!(
        launch.executable_sha256,
        ketchup_core::graph::sha256_hex(&fs::read(&launch.executable).unwrap())
    );
}
