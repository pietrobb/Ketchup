mod harness;

use harness::{ScriptedAssistantTransport, Shell};
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::assistant_sidecar::AssistantChatResult;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn ai_house_schema_36_fixture_opens_from_file_menu() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ai_house_orbit.ketchup");
    let loaded = ketchup_core::persistence::load_file(&fixture).unwrap();
    assert_eq!(loaded.source_schema(), 36);
    assert_eq!(
        loaded.disposition(),
        ketchup_core::persistence::LoadDisposition::EditableLossless
    );
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    shell.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(shell.app().document_path(), Some(fixture.as_path()));
    assert_eq!(shell.app().document_snapshot().definitions().count(), 64);
    assert_eq!(shell.app().document_snapshot().occurrences().count(), 64);
    assert!(!shell.app().is_dirty());
}

#[test]
fn ai_house_fixture_orbits_interactively_with_exact_geometry() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ai_house_orbit.ketchup");
    assert_eq!(std::fs::metadata(&fixture).unwrap().len(), 123_122);

    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);

    let snapshot = shell.app().document_snapshot();
    let definitions = snapshot.definitions().count();
    assert_eq!(definitions, 64);
    assert_eq!(snapshot.occurrences().count(), 64);

    let worker_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let worker = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(worker_name);
    shell.app_mut().connect_exact_worker(&worker).unwrap();
    for _ in 0..500 {
        shell.step();
        if shell.app().exact_render_body_count() == definitions {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(shell.app().exact_render_body_count(), definitions);

    shell.app_mut().enable_headless_instanced_scene();
    shell.settle();
    let pointer = shell.viewport_rect().center();
    let orbit_started = Instant::now();
    shell.orbit_drag(pointer, eframe::egui::Vec2::new(3.0, -2.0), 20);
    let orbit_elapsed = orbit_started.elapsed();
    assert!(
        orbit_elapsed < Duration::from_secs(1),
        "twenty orbit frames of the exact AI house fixture took {orbit_elapsed:?}"
    );
}

#[test]
fn ai_house_follow_up_fits_the_sidecar_request_envelope() {
    let query = "Urob normálnu sedlovú strechu namiesto tých stupňov";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Strechu viem opraviť.".to_owned(),
            model_intent: None,
        },
    )]));
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ai_house_orbit.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs_and_assistant_transport(dialogs, transport.clone());
    shell.click_menu_command("menu-file", AppCommand::Open);

    let worker_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let worker = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(worker_name);
    shell.app_mut().connect_exact_worker(&worker).unwrap();
    for _ in 0..500 {
        shell.step();
        if shell.app().exact_render_body_count() == 64 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(shell.app().exact_render_body_count(), 64);

    let input_label = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input_label);
    shell.type_text(query);
    shell.press_key(eframe::egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if !transport.contexts().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let context = transport.contexts().into_iter().next().unwrap();
    let encoded = serde_json::to_vec(&context).unwrap();
    assert!(
        encoded.len() <= 24 * 1024,
        "AI house provider context uses {} bytes",
        encoded.len()
    );
    assert_eq!(context["context_complete"], false);
    assert_eq!(context["boxes_complete"], false);
    assert_eq!(context["boxes"], serde_json::json!([]));
    assert_eq!(context["occurrences"].as_array().unwrap().len(), 64);
    assert!(!context["conversation"].as_array().unwrap().is_empty());
    assert!(context["state_view"]["content"].as_str().unwrap().len() <= 1024);
}
