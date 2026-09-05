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

/// CPU-side equivalent of the GPU-enabled desktop path, not a GPU benchmark:
/// egui_kittest builds the real instanced scene but never executes paint callbacks.
/// Keep separate from the legacy 64-body fixture and use default OAuth features.
#[test]
fn garden_studio_exact_load_idle_and_orbit_performance() {
    use eframe::egui::{Event, Modifiers, PointerButton, Vec2};
    use ketchup_app::KetchupApp;

    const BODIES: usize = 93;
    const FRAMES: usize = 20;
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/garden-studio.ketchup");
    let worker = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(if cfg!(windows) {
            "ketchup-exact-worker.exe"
        } else {
            "ketchup-exact-worker"
        });
    assert!(
        worker.is_file(),
        "missing exact worker: {}",
        worker.display()
    );
    eprintln!(
        "garden path=headless-instanced debug={} private-oauth={} fixture_bytes={} worker={}",
        cfg!(debug_assertions),
        cfg!(feature = "private-oauth"),
        std::fs::metadata(&fixture).unwrap().len(),
        worker.display()
    );

    eprintln!("garden stage=app-create begin");
    let started = Instant::now();
    let mut app = KetchupApp::new().with_dialogs(Box::new(ScriptedFileDialogs::new()));
    app.enable_headless_instanced_scene();
    eprintln!("garden stage=app-create elapsed={:?}", started.elapsed());

    eprintln!("garden stage=open-document begin");
    let started = Instant::now();
    assert!(app.open_document_path(&fixture));
    eprintln!("garden stage=open-document elapsed={:?}", started.elapsed());
    assert_eq!(app.document_path(), Some(fixture.as_path()));
    assert_eq!(app.document_snapshot().definitions().count(), BODIES);
    assert_eq!(app.document_snapshot().occurrences().count(), BODIES);
    assert!(!app.is_dirty());
    let digest = app.canonical_digest();
    let revision = app.document_revision();

    eprintln!("garden stage=worker-connect begin");
    let started = Instant::now();
    app.connect_exact_worker(&worker).unwrap();
    eprintln!(
        "garden stage=worker-connect elapsed={:?}",
        started.elapsed()
    );

    // Explicit steps rather than run/settle: exactly one input frame per sample,
    // and a wall-clock deadline instead of an arbitrary number of worker polls.
    eprintln!("garden stage=harness-first-frame begin");
    let exact_started = Instant::now();
    let mut ui = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(
            |ctx, app: &mut KetchupApp| {
                let started = Instant::now();
                app.ui(ctx);
                eprintln!("garden stage=app-ui-pass elapsed={:?}", started.elapsed());
            },
            app,
        );
    ui.ctx.style_mut(|style| style.animation_time = 0.0);
    eprintln!(
        "garden stage=harness-first-frame elapsed={:?}",
        exact_started.elapsed()
    );
    eprintln!("garden stage=exact-publication begin");
    let mut poll = 0;
    while ui.state().exact_render_body_count() != BODIES {
        assert!(
            exact_started.elapsed() < Duration::from_secs(120),
            "garden exact publication timed out: {}/93 bodies after {:?}",
            ui.state().exact_render_body_count(),
            exact_started.elapsed()
        );
        let started = Instant::now();
        ui.step();
        let elapsed = started.elapsed();
        poll += 1;
        if elapsed > Duration::from_millis(250) || poll % 100 == 0 {
            eprintln!(
                "garden stage=exact-poll frame={poll} elapsed={elapsed:?} bodies={}/93 wall={:?}",
                ui.state().exact_render_body_count(),
                exact_started.elapsed()
            );
        }
        if ui.state().exact_render_body_count() != BODIES {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    eprintln!(
        "garden stage=exact-ready elapsed={:?} polls={poll} bodies={} triangles={}",
        exact_started.elapsed(),
        ui.state().exact_render_body_count(),
        ui.state().exact_render_triangle_count()
    );
    assert!(ui.state().exact_render_triangle_count() > 0);

    let started = Instant::now();
    for _ in 0..3 {
        ui.step();
    }
    eprintln!(
        "garden stage=warmup frames=3 elapsed={:?}",
        started.elapsed()
    );
    assert!(ui.state().instanced_scene_triangle_count() > 0);

    let mut idle = Vec::with_capacity(FRAMES);
    eprintln!("garden stage=idle begin frames={FRAMES}");
    for frame in 0..FRAMES {
        let started = Instant::now();
        ui.step();
        let elapsed = started.elapsed();
        idle.push(elapsed);
        eprintln!("garden stage=idle frame={} elapsed={elapsed:?}", frame + 1);
    }
    let idle_total = report_garden_frames("idle", &idle);

    let mut pointer = ui.state().viewport_rect().unwrap().center();
    ui.input_mut().events.push(Event::PointerMoved(pointer));
    ui.step();
    ui.input_mut().events.push(Event::PointerButton {
        pos: pointer,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    ui.step();
    let probe = ketchup_interaction::Vec3::new(1000.0, 2000.0, 3000.0);
    let before = ui.state().viewport_position(probe).unwrap();
    let mut orbit = Vec::with_capacity(FRAMES);
    eprintln!("garden stage=orbit begin frames={FRAMES}");
    for frame in 0..FRAMES {
        pointer += Vec2::new(3.0, -2.0);
        ui.input_mut().events.push(Event::PointerMoved(pointer));
        let started = Instant::now();
        ui.step();
        let elapsed = started.elapsed();
        orbit.push(elapsed);
        eprintln!("garden stage=orbit frame={} elapsed={elapsed:?}", frame + 1);
    }
    let orbit_total = report_garden_frames("orbit", &orbit);
    let after = ui.state().viewport_position(probe).unwrap();
    ui.input_mut().events.push(Event::PointerButton {
        pos: pointer,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    ui.step();

    assert!(
        before.distance(after) > 1.0,
        "orbit must actually move the camera"
    );
    assert_eq!(ui.state().exact_render_body_count(), BODIES);
    assert!(ui.state().instanced_scene_triangle_count() > 0);
    assert_eq!(ui.state().canonical_digest(), digest);
    assert_eq!(ui.state().document_revision(), revision);
    assert!(!ui.state().is_dirty());
    // Same generous 20-frame responsiveness guard as the legacy regression.
    // Report both series before asserting so a slow baseline remains diagnostic.
    assert!(
        idle_total < Duration::from_secs(1) && orbit_total < Duration::from_secs(1),
        "garden 20-frame budget exceeded: idle={idle_total:?}, orbit={orbit_total:?} (budget=1s each)"
    );
}

fn report_garden_frames(stage: &str, samples: &[Duration]) -> Duration {
    let total: Duration = samples.iter().copied().sum();
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let count = u32::try_from(samples.len()).unwrap();
    eprintln!(
        "garden summary stage={stage} frames={count} total={total:?} mean={:?} p95={:?} max={:?}",
        total / count,
        sorted[(samples.len() * 95).div_ceil(100) - 1],
        sorted.last().unwrap()
    );
    total
}
