use crate::harness;

use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use std::time::{Duration, Instant};

fn exact_worker_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

#[test]
fn ai_house_schema_36_fixture_opens_from_file_menu() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("ai_house_orbit.ketchup");
    std::fs::write(&fixture, include_bytes!("fixtures/ai_house_orbit.ketchup")).unwrap();
    let loaded = ketchup_model::persistence::load_file(&fixture).unwrap();
    assert_eq!(loaded.source_schema(), 36);
    assert_eq!(
        loaded.disposition(),
        ketchup_model::persistence::LoadDisposition::EditableLossless
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
    orbit_ai_house();
}

/// The time budgets of this file hold in release, nightly; the tests above
/// them check what happened (review 2026-10-17 TST-3).
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_ai_house_fixture_orbits_twenty_frames_in_a_second() {
    let orbit = orbit_ai_house();
    assert!(
        orbit < Duration::from_secs(1),
        "twenty orbit frames of the exact AI house fixture took {orbit:?}"
    );
}

/// Opens the 64-body AI house with exact geometry and orbits it 20 frames;
/// the time of the orbit.
fn orbit_ai_house() -> Duration {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("ai_house_orbit.ketchup");
    std::fs::write(&fixture, include_bytes!("fixtures/ai_house_orbit.ketchup")).unwrap();
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

    let worker = exact_worker_path();
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
    let probe = ketchup_interaction::Vec3::new(1000.0, 2000.0, 3000.0);
    let before = shell.app().viewport_position(probe).unwrap();
    let orbit_started = Instant::now();
    shell.orbit_drag(pointer, eframe::egui::Vec2::new(3.0, -2.0), 20);
    let orbit = orbit_started.elapsed();
    let after = shell.app().viewport_position(probe).unwrap();
    assert!(
        before.distance(after) > 1.0,
        "orbit must actually move the camera"
    );
    assert_eq!(shell.app().exact_render_body_count(), definitions);
    orbit
}

/// CPU-side equivalent of the GPU-enabled desktop path, not a GPU benchmark:
/// egui_kittest builds the real instanced scene but never executes paint callbacks.
/// Keep separate from the legacy 64-body fixture.
#[test]
fn garden_studio_exact_load_idle_and_orbit_performance() {
    exact_load_idle_and_orbit_performance("garden-studio.ketchup", 93);
}

#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_garden_studio_idles_hovers_and_orbits_within_budget() {
    perf_frames_within_budget("garden-studio.ketchup", 93);
}

/// Program-built wardrobe with drawers, dowel holes, a table and a chair
/// (82 bodies, ~800 features): the many drilled holes make dense exact meshes.
#[test]
fn wardrobe_exact_load_idle_and_orbit_performance() {
    exact_load_idle_and_orbit_performance("wardrobe-chair-table.ketchup", 82);
}

#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_wardrobe_idles_hovers_and_orbits_within_budget() {
    perf_frames_within_budget("wardrobe-chair-table.ketchup", 82);
}

/// The same generous 20-frame responsiveness guard as the AI house. Hover gets
/// 2 s: rebuilding the exact registry and matching every mesh edge against
/// every B-rep edge took >1 s in release before; the wardrobe now takes ~0.16 s.
fn perf_frames_within_budget(example: &str, bodies: usize) {
    let [idle, hover, orbit] = exact_load_idle_and_orbit_performance(example, bodies);
    assert!(
        idle < Duration::from_secs(1)
            && hover < Duration::from_secs(2)
            && orbit < Duration::from_secs(1),
        "{example} 20-frame budget exceeded: idle={idle:?}, hover={hover:?}, orbit={orbit:?} (budget=1s, hover 2s)"
    );
}

/// Opens `example`, publishes its exact bodies and takes 20 idle, hover and
/// orbit frames each; their totals.
fn exact_load_idle_and_orbit_performance(example: &str, bodies: usize) -> [Duration; 3] {
    use eframe::egui::{Event, Modifiers, PointerButton, Vec2};
    use ketchup_app::KetchupApp;

    const FRAMES: usize = 20;
    let _turn = crate::integration_support::file_turn();
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join(example);
    std::fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(example),
        &fixture,
    )
    .unwrap();
    let worker = exact_worker_path();
    eprintln!(
        "{example} path=headless-instanced debug={} fixture_bytes={} worker={}",
        cfg!(debug_assertions),
        std::fs::metadata(&fixture).unwrap().len(),
        worker.display()
    );

    eprintln!("{example} stage=app-create begin");
    let started = Instant::now();
    let mut app = KetchupApp::new().with_dialogs(Box::new(ScriptedFileDialogs::new()));
    app.enable_headless_instanced_scene();
    eprintln!("{example} stage=app-create elapsed={:?}", started.elapsed());

    eprintln!("{example} stage=open-document begin");
    let started = Instant::now();
    assert!(app.open_document_path(&fixture));
    eprintln!(
        "{example} stage=open-document elapsed={:?}",
        started.elapsed()
    );
    assert_eq!(app.document_path(), Some(fixture.as_path()));
    assert_eq!(app.document_snapshot().definitions().count(), bodies);
    assert_eq!(app.document_snapshot().occurrences().count(), bodies);
    assert!(!app.is_dirty());
    let digest = app.canonical_digest();
    let revision = app.document_revision();

    eprintln!("{example} stage=worker-connect begin");
    let started = Instant::now();
    app.connect_exact_worker(&worker).unwrap();
    eprintln!(
        "{example} stage=worker-connect elapsed={:?}",
        started.elapsed()
    );

    // Explicit steps rather than run/settle: exactly one input frame per sample,
    // and a wall-clock deadline instead of an arbitrary number of worker polls.
    eprintln!("{example} stage=harness-first-frame begin");
    let exact_started = Instant::now();
    let mut ui = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(
            |ctx, app: &mut KetchupApp| {
                let started = Instant::now();
                app.ui(ctx);
                eprintln!(
                    "{example} stage=app-ui-pass elapsed={:?}",
                    started.elapsed()
                );
            },
            app,
        );
    ui.ctx.style_mut(|style| style.animation_time = 0.0);
    eprintln!(
        "{example} stage=harness-first-frame elapsed={:?}",
        exact_started.elapsed()
    );
    eprintln!("{example} stage=exact-publication begin");
    let mut poll = 0;
    let deadline = exact_started + Duration::from_secs(120);
    while ui.state().exact_render_body_count() != bodies {
        assert!(
            Instant::now() < deadline,
            "{example} exact publication timed out: {}/{bodies} bodies after {:?}",
            ui.state().exact_render_body_count(),
            exact_started.elapsed()
        );
        let started = Instant::now();
        ui.step();
        let elapsed = started.elapsed();
        poll += 1;
        if elapsed > Duration::from_millis(250) || poll % 100 == 0 {
            eprintln!(
                "{example} stage=exact-poll frame={poll} elapsed={elapsed:?} bodies={}/{bodies} wall={:?}",
                ui.state().exact_render_body_count(),
                exact_started.elapsed()
            );
        }
        if ui.state().exact_render_body_count() != bodies {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    eprintln!(
        "{example} stage=exact-ready elapsed={:?} polls={poll} bodies={} triangles={}",
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
        "{example} stage=warmup frames=3 elapsed={:?}",
        started.elapsed()
    );
    assert!(ui.state().instanced_scene_triangle_count() > 0);

    let mut idle = Vec::with_capacity(FRAMES);
    eprintln!("{example} stage=idle begin frames={FRAMES}");
    for frame in 0..FRAMES {
        let started = Instant::now();
        ui.step();
        let elapsed = started.elapsed();
        idle.push(elapsed);
        eprintln!(
            "{example} stage=idle frame={} elapsed={elapsed:?}",
            frame + 1
        );
    }
    let idle_total = report_frames(example, "idle", &idle);

    // Moving the pointer over the model hovers bodies, which picks, snaps and
    // draws a CPU highlight every frame; orbit starts and ends this way.
    let mut pointer = ui.state().viewport_rect().unwrap().center();
    let mut hover = Vec::with_capacity(FRAMES);
    eprintln!("{example} stage=hover begin frames={FRAMES}");
    for frame in 0..FRAMES {
        let step = frame as f32;
        let hovered = pointer + Vec2::new(-60.0 + 6.0 * step, 40.0 - 4.0 * step);
        ui.input_mut().events.push(Event::PointerMoved(hovered));
        let started = Instant::now();
        ui.step();
        let elapsed = started.elapsed();
        hover.push(elapsed);
        eprintln!(
            "{example} stage=hover frame={} elapsed={elapsed:?}",
            frame + 1
        );
    }
    let hover_total = report_frames(example, "hover", &hover);
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
    eprintln!("{example} stage=orbit begin frames={FRAMES}");
    for frame in 0..FRAMES {
        pointer += Vec2::new(3.0, -2.0);
        ui.input_mut().events.push(Event::PointerMoved(pointer));
        let started = Instant::now();
        ui.step();
        let elapsed = started.elapsed();
        orbit.push(elapsed);
        eprintln!(
            "{example} stage=orbit frame={} elapsed={elapsed:?}",
            frame + 1
        );
    }
    let orbit_total = report_frames(example, "orbit", &orbit);
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
    assert_eq!(ui.state().exact_render_body_count(), bodies);
    assert!(ui.state().instanced_scene_triangle_count() > 0);
    assert_eq!(ui.state().canonical_digest(), digest);
    assert_eq!(ui.state().document_revision(), revision);
    assert!(!ui.state().is_dirty());
    [idle_total, hover_total, orbit_total]
}

fn report_frames(example: &str, stage: &str, samples: &[Duration]) -> Duration {
    let total: Duration = samples.iter().copied().sum();
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let count = u32::try_from(samples.len()).unwrap();
    eprintln!(
        "{example} summary stage={stage} frames={count} total={total:?} mean={:?} p95={:?} max={:?}",
        total / count,
        sorted[(samples.len() * 95).div_ceil(100) - 1],
        sorted.last().unwrap()
    );
    total
}
