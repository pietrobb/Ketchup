//! Opt-in live proof: production OAuth binary must actually call validator tools.

mod harness;

use eframe::egui::accesskit::Role;
use harness::Shell;
use ketchup_app::assistant_sidecar_command;
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantBoxIntent, AssistantCapability, AssistantDistribution,
    AssistantHandshake, AssistantModelIntent, AssistantTranslationIntent,
};
use ketchup_scheduler::assistant::AssistantProcessClient;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

fn timber(name: &str, size_mm: [f64; 3], origin_mm: [f64; 3]) -> AssistantBoxIntent {
    AssistantBoxIntent {
        name: name.to_owned(),
        size_mm,
        origin_mm,
        subtract_boxes: Vec::new(),
    }
}

fn live_verdict(context: &Value, expected_state: &str) -> Value {
    let (program, arguments) = assistant_sidecar_command(AssistantDistribution::PrivateOauth)
        .expect("a production private OAuth binary must be configured for this opt-in test");
    let handshake = AssistantHandshake {
        protocol_version: ASSISTANT_PROTOCOL_VERSION,
        distribution: AssistantDistribution::PrivateOauth,
        provider: "codex-oauth".to_owned(),
        model: "gpt-5.6-sol".to_owned(),
        capabilities: BTreeSet::from([
            AssistantCapability::Chat,
            AssistantCapability::DebugObservability,
            AssistantCapability::LocalMemory,
            AssistantCapability::QueryDocument,
            AssistantCapability::ProposeWorkflowIntent,
        ]),
    };
    let mut client =
        AssistantProcessClient::spawn(program, &arguments, handshake, Duration::from_secs(300))
            .expect("the production handshake must succeed");
    let exchange = client
        .chat_exchange(
            "live-gravity-support",
            "Read-only check. First call list_validators, then call run_validators for gravity_support only. Report the returned state, unsupported part names and assumptions in English. Do not propose any edits. Do not infer the verdict instead of calling the tools.",
            context,
        )
        .expect("live OAuth validation must succeed");
    client.shutdown().expect("clean sidecar shutdown");
    assert!(exchange.result.model_intent.is_none());
    assert!(exchange.cad_edit_program.is_none());
    let diagnostics = exchange.diagnostics.expect("tool evidence is mandatory");
    let input = diagnostics.request_payload["input"]
        .as_array()
        .expect("Responses input contains the tool trace");
    let calls = input
        .iter()
        .filter(|item| item["type"] == "function_call")
        .collect::<Vec<_>>();
    let mut gravity = None;
    for name in ["list_validators", "run_validators"] {
        let call = calls
            .iter()
            .find(|call| call["name"] == name)
            .unwrap_or_else(|| panic!("missing actual {name} call; prose is not proof"));
        let output = input
            .iter()
            .find(|item| {
                item["type"] == "function_call_output" && item["call_id"] == call["call_id"]
            })
            .expect("every validator call must have a captured tool result");
        let result: Value =
            serde_json::from_str(output["output"].as_str().unwrap()).expect("tool output is JSON");
        assert_eq!(result["revision"], context["validation"]["revision"]);
        if name == "run_validators" {
            assert_eq!(
                result["canonical_digest"],
                context["validation"]["canonical_digest"]
            );
            gravity = result["results"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["validator"] == "gravity_support")
                .cloned();
        }
    }
    let gravity = gravity.expect("gravity_support was actually requested");
    assert_eq!(gravity["state"], expected_state);
    assert!(gravity["evidence_complete"].as_bool().unwrap());
    assert!(gravity["not_evaluated_reason"].is_null());
    assert!(!gravity["assumptions"].as_array().unwrap().is_empty());
    assert!(
        exchange
            .result
            .message
            .to_lowercase()
            .contains(expected_state)
    );
    if expected_state == "failed" {
        assert!(
            gravity["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|issue| issue["name"] == "Ridge beam")
        );
        assert!(exchange.result.message.contains("Ridge beam"));
    } else {
        assert!(gravity["issues"].as_array().unwrap().is_empty());
    }
    println!(
        "LIVE_VALIDATOR_EVIDENCE {}",
        serde_json::json!({
            "model": diagnostics.model,
            "message": exchange.result.message,
            "input_tokens": diagnostics.input_tokens,
            "output_tokens": diagnostics.output_tokens,
            "trace": input.iter().filter(|item| item["type"] == "function_call" || item["type"] == "function_call_output").collect::<Vec<_>>(),
        })
    );
    gravity
}

#[test]
#[ignore = "requires the production OAuth binary, login and live GPT-5.6 requests"]
fn live_oauth_tools_reject_floating_ridge_and_accept_supported_ridge() {
    let mut shell = Shell::new();
    let mut intent = AssistantModelIntent {
        replace_scene: true,
        boxes: vec![
            timber("Foundation", [4000.0, 400.0, 300.0], [0.0, 0.0, 0.0]),
            timber("Post left", [200.0, 200.0, 2500.0], [0.0, 0.0, 300.0]),
            timber("Post right", [200.0, 200.0, 2500.0], [3800.0, 0.0, 300.0]),
            timber("Ridge beam", [4000.0, 200.0, 200.0], [0.0, 0.0, 3500.0]),
        ],
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
    };
    assert!(
        shell
            .app_mut()
            .prepare_assistant_model_intent(intent.clone())
    );
    assert!(shell.app_mut().confirm_assistant_proposal());
    shell.settle();
    shell.click_role_and_label(Role::Button, &shell.catalog().text("assembly-title"));
    let preview = shell.catalog().format(
        "assembly-preview-ground",
        &BTreeMap::from([("name", "Foundation".to_owned())]),
    );
    shell.click_button_label(&preview);
    shell.click_button_label(&shell.catalog().text("assembly-confirm-preview"));
    shell.click_role_and_label(Role::Button, &shell.catalog().text("assembly-title"));
    shell.settle();
    for expected_state in ["failed", "passed"] {
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo = shell.app().undo_step_count();
        live_verdict(&shell.app().assistant_context(), expected_state);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo);
        if expected_state == "failed" {
            let ridge = shell
                .app()
                .document_snapshot()
                .scene_query()
                .into_iter()
                .find(|occurrence| occurrence.occurrence_name == "Ridge beam")
                .unwrap()
                .occurrence_id
                .0;
            intent.replace_scene = false;
            intent.boxes.clear();
            intent.translations = vec![AssistantTranslationIntent {
                occurrence_id: ridge,
                delta_mm: [0.0, 0.0, -700.0],
            }];
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_model_intent(intent.clone())
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
            shell.settle();
        }
    }
}
