//! Handshake coverage against the sidecars the shipped app actually launches.
//!
//! The headless assistant suites talk to purpose-written mock sidecars, so they
//! stayed green while the real private OAuth build silently fell behind protocol
//! version 3 and rejected every request in the UI with "unsupported protocol
//! version". These tests spawn the same executables the product resolves at
//! runtime and require a completed handshake.

use ketchup_app::assistant_sidecar_command;
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantCapability, AssistantDistribution, AssistantHandshake,
};
use ketchup_scheduler::assistant::AssistantProcessClient;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

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

fn complete_handshake(program: PathBuf, arguments: Vec<OsString>, handshake: AssistantHandshake) {
    let mut client =
        AssistantProcessClient::spawn(&program, &arguments, handshake, HANDSHAKE_TIMEOUT)
            .unwrap_or_else(|error| {
                panic!(
                    "{} rejected the production handshake: {error}",
                    program.display()
                )
            });
    client.shutdown().expect("sidecar shutdown");
}

#[test]
fn public_sidecar_script_completes_the_production_handshake() {
    let script = repository_root().join("sdk/python/ketchup_assistant.py");
    assert!(script.is_file(), "{} is missing", script.display());
    let python = std::env::var_os("KETCHUP_PYTHON").unwrap_or_else(|| {
        OsString::from(if cfg!(windows) {
            "python.exe"
        } else {
            "python3"
        })
    });
    complete_handshake(
        PathBuf::from(python),
        vec![script.into_os_string()],
        handshake(
            AssistantDistribution::PublicApi,
            "anthropic-api",
            "claude-sonnet-4-6",
        ),
    );
}

#[test]
fn private_oauth_sidecar_binary_completes_the_production_handshake() {
    let Ok((program, arguments)) = assistant_sidecar_command(AssistantDistribution::PrivateOauth)
    else {
        eprintln!("skipped: no private OAuth sidecar is configured on this machine");
        return;
    };
    complete_handshake(
        program,
        arguments,
        handshake(
            AssistantDistribution::PrivateOauth,
            "codex-oauth",
            "gpt-5.6-sol",
        ),
    );
}
