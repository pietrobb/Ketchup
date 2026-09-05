//! The Assistant must reach the validators the same way the operator does.
//!
//! These tests drive the *real* sidecar tool code (`sdk/python/ketchup_assistant.py`
//! and the shared protocol module both distributions load) with a *real* host
//! document context produced by the app. Nothing here is a fixture: if the host
//! stops publishing the validator catalog, or the sidecar stops understanding it,
//! the coupling breaks here instead of in the operator's UI.

mod harness;

use harness::Shell;
use ketchup_app::KetchupApp;
use ketchup_core::assistant_sidecar::{AssistantBoxIntent, AssistantModelIntent};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

fn empty_intent(boxes: Vec<AssistantBoxIntent>) -> AssistantModelIntent {
    AssistantModelIntent {
        replace_scene: true,
        boxes,
        translations: Vec::new(),
        rotations: Vec::new(),
        profile_translations: Vec::new(),
        parameter_edits: Vec::new(),
        linear_arrays: Vec::new(),
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: Vec::new(),
    }
}

/// Two columns driven into each other: one validator must fail with concrete
/// part names, while the role-driven validators must stay honestly
/// unevaluated. The Assistant has to be able to report both truthfully.
fn build_a_scene_with_a_real_finding(shell: &mut Shell) {
    assert!(
        shell
            .app_mut()
            .prepare_assistant_model_intent(empty_intent(vec![
                AssistantBoxIntent {
                    name: "Column A".to_owned(),
                    size_mm: [400.0, 400.0, 2_500.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Column B".to_owned(),
                    size_mm: [400.0, 400.0, 2_500.0],
                    origin_mm: [200.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ]))
    );
    assert!(shell.app_mut().confirm_assistant_proposal());
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn python() -> OsString {
    std::env::var_os("KETCHUP_PYTHON").unwrap_or_else(|| {
        OsString::from(if cfg!(windows) {
            "python.exe"
        } else {
            "python3"
        })
    })
}

const DRIVER: &str = r#"import importlib.util
import json
import sys

spec = importlib.util.spec_from_file_location("ketchup_assistant", sys.argv[1])
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)

with open(sys.argv[2], encoding="utf-8") as handle:
    message = handle.read()

listed = module._read_only_tool_result(message, "list_validators", {})
ran = module._read_only_tool_result(
    message,
    "run_validators",
    {"validators": [entry["id"] for entry in listed["validators"]]},
)
sys.stdout.write(json.dumps({"listed": listed, "ran": ran}))
"#;

/// Ask the real sidecar tool implementation what it sees in a real host context.
fn sidecar_validator_tools(context: &serde_json::Value, question: &str) -> serde_json::Value {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let driver = temporary.path().join("driver.py");
    std::fs::write(&driver, DRIVER).expect("driver script");
    let message_path = temporary.path().join("message.txt");
    let message = format!(
        "<document-context>{}</document-context>\n\n{question}",
        serde_json::to_string(context).expect("context is serializable")
    );
    std::fs::write(&message_path, message).expect("message file");

    let sidecar = repository_root().join("sdk/python/ketchup_assistant.py");
    assert!(sidecar.is_file(), "{} is missing", sidecar.display());
    let output = Command::new(python())
        .arg(&driver)
        .arg(&sidecar)
        .arg(&message_path)
        .output()
        .expect("the sidecar tool driver runs");
    assert!(
        output.status.success(),
        "the sidecar rejected the host context: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("the sidecar returns JSON")
}

#[test]
fn the_assistant_can_enumerate_every_validator_the_operator_can_run() {
    let mut shell = Shell::new();
    build_a_scene_with_a_real_finding(&mut shell);
    shell.settle();

    let context = shell.app().assistant_context();
    let tools = sidecar_validator_tools(&context, "Which checks can you run on this model?");
    let listed = &tools["listed"]["validators"];

    let ids = listed
        .as_array()
        .expect("the sidecar lists validators")
        .iter()
        .map(|entry| entry["id"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        KetchupApp::validator_ids()
            .iter()
            .map(|validator| (*validator).to_owned())
            .collect::<Vec<_>>(),
        "the Assistant must see exactly the validators the panel offers"
    );
    for entry in listed.as_array().unwrap() {
        let checks = entry["checks"].as_str().unwrap_or_default();
        assert!(
            checks.len() > 20,
            "{} must tell the Assistant what it checks, got {checks:?}",
            entry["id"]
        );
        assert_eq!(
            entry["already_run_on_this_revision"],
            serde_json::Value::Bool(true)
        );
    }
    assert_eq!(
        tools["listed"]["revision"].as_u64(),
        Some(shell.app().document_revision()),
        "the catalog must be bound to the revision it was read from"
    );
}

#[test]
fn the_assistant_reads_findings_that_name_the_offending_parts() {
    let mut shell = Shell::new();
    build_a_scene_with_a_real_finding(&mut shell);
    shell.settle();

    let revision_before = shell.app().document_revision();
    let digest_before = shell.app().canonical_digest();
    let undo_before = shell.app().undo_step_count();

    let context = shell.app().assistant_context();
    let tools = sidecar_validator_tools(&context, "Is anything unsupported?");
    let results = tools["ran"]["results"]
        .as_array()
        .expect("the sidecar returns one result per requested validator")
        .clone();
    assert_eq!(results.len(), KetchupApp::validator_ids().len());
    assert_eq!(
        tools["ran"]["canonical_digest"].as_str(),
        Some(digest_before.as_str()),
        "findings must be bound to the model they were read from"
    );

    let collision = results
        .iter()
        .find(|result| result["validator"] == "collision")
        .expect("collision must be reachable as a tool");
    assert_eq!(collision["state"], "failed");
    let names = collision["issues"]
        .as_array()
        .expect("a failing validator must carry its findings")
        .iter()
        .flat_map(|issue| [issue["left_name"].as_str(), issue["right_name"].as_str()])
        .flatten()
        .collect::<Vec<_>>();
    assert!(
        names.contains(&"Column A") && names.contains(&"Column B"),
        "the finding must name the concrete parts, got {names:?}"
    );

    // Honesty over optimism: a validator that needs explicit roles this model
    // does not carry must say it was not evaluated, never that the model passed.
    let gravity = results
        .iter()
        .find(|result| result["validator"] == "gravity_support")
        .expect("gravity support must be reachable as a tool");
    assert_eq!(gravity["state"], "not_evaluated");
    assert!(
        gravity["not_evaluated_reason"].is_string(),
        "an unevaluated validator must say why, got {gravity}"
    );

    // Every result is honest about its own evidence: either it is complete or it
    // says why it is not. Nothing here silently claims the model is fine.
    for result in &results {
        assert!(result["evidence_complete"].is_boolean());
        if result["evidence_complete"] == serde_json::Value::Bool(false) {
            assert!(
                result["not_evaluated_reason"].is_string()
                    || result["state"] != serde_json::Value::String("passed".to_owned()),
                "{} claimed a pass on incomplete evidence",
                result["validator"]
            );
        }
    }

    // Reading validators is observation, not a change.
    assert_eq!(shell.app().document_revision(), revision_before);
    assert_eq!(shell.app().canonical_digest(), digest_before);
    assert_eq!(shell.app().undo_step_count(), undo_before);
}
