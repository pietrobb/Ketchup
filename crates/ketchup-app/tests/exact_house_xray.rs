//! Opt-in native exact-worker CPU regression; no desktop window or GPU execution.
//! Run with default OAuth features after building ketchup-exact-worker in the same
//! profile: cargo test -p ketchup-app --release --test exact_house_xray -- --ignored --nocapture

use eframe::egui::{Event, Modifiers, PointerButton, Vec2, epaint::Primitive};
use ketchup_app::{KetchupApp, dialogs::ScriptedFileDialogs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BODIES: usize = 140;
const FRAMES: usize = 20;
const DEADLINE: Duration = Duration::from_secs(180);
const FRAME_BUDGET: Duration = Duration::from_millis(100);

struct BenchState {
    app: KetchupApp,
    ui_time: Duration,
}

#[derive(Clone, Copy)]
struct Sample {
    ui: Duration,
    preparation: Duration,
    tessellation: Duration,
    total: Duration,
    vertices: usize,
    callbacks: usize,
}

fn step(ui: &mut egui_kittest::Harness<'_, BenchState>) -> Sample {
    ui.state_mut().ui_time = Duration::ZERO;
    let started = Instant::now();
    ui.step();
    let preparation = started.elapsed();
    // Cloning is a harness-only expense: native egui consumes FullOutput.shapes.
    // Exclude it, and primitive inspection below, from the CPU-frame measurement.
    let shapes = ui.output().shapes.clone();
    let started = Instant::now();
    let primitives = ui.ctx.tessellate(shapes, ui.output().pixels_per_point);
    let tessellation = started.elapsed();
    let mut vertices = 0;
    let mut callbacks = 0;
    for primitive in &primitives {
        match &primitive.primitive {
            Primitive::Mesh(mesh) => vertices += mesh.vertices.len(),
            Primitive::Callback(_) => callbacks += 1,
        }
    }
    Sample {
        ui: ui.state().ui_time,
        preparation,
        tessellation,
        total: preparation + tessellation,
        vertices,
        callbacks,
    }
}

fn report(mode: &str, edges: bool, motion: &str, samples: &[Sample]) -> Duration {
    assert_eq!(samples.len(), FRAMES);
    let mut frame_p95 = Duration::ZERO;
    for (metric, select) in [
        ("app-ui", (|s: &Sample| s.ui) as fn(&Sample) -> Duration),
        ("preparation", |s: &Sample| s.preparation),
        ("tessellation", |s: &Sample| s.tessellation),
        ("cpu-frame", |s: &Sample| s.total),
    ] {
        let mut sorted: Vec<_> = samples.iter().map(select).collect();
        sorted.sort_unstable();
        let median = (sorted[FRAMES / 2 - 1] + sorted[FRAMES / 2]) / 2;
        let p95 = sorted[(FRAMES * 95).div_ceil(100) - 1];
        eprintln!(
            "exact-house mode={mode} edges={edges} motion={motion} metric={metric} frames={FRAMES} median_ms={:.3} p95_ms={:.3} max_ms={:.3}",
            median.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0,
            sorted[FRAMES - 1].as_secs_f64() * 1000.0,
        );
        if metric == "cpu-frame" {
            frame_p95 = p95;
        }
    }
    eprintln!(
        "exact-house mode={mode} edges={edges} motion={motion} egui_vertices_max={} callbacks_max={}",
        samples.iter().map(|s| s.vertices).max().unwrap(),
        samples.iter().map(|s| s.callbacks).max().unwrap(),
    );
    frame_p95
}

#[test]
#[ignore = "native exact worker plus external 140-body example; explicit CPU benchmark opt-in"]
fn exact_house_shaded_and_xray_idle_orbit_cpu_regression() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/garden-studio-exact.ketchup");
    let original_bytes = std::fs::read(&fixture).expect("140-body exact house fixture is required");
    let worker = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(if cfg!(windows) {
            "ketchup-exact-worker.exe"
        } else {
            "ketchup-exact-worker"
        });
    assert!(
        worker.is_file(),
        "build the exact worker first: {}",
        worker.display()
    );
    eprintln!(
        "exact-house debug={} private-oauth={} fixture={} bytes={} worker={} deadline_s=180 path=headless-instanced CPU-only",
        cfg!(debug_assertions),
        cfg!(feature = "private-oauth"),
        fixture.display(),
        original_bytes.len(),
        worker.display(),
    );
    let mut app = KetchupApp::new().with_dialogs(Box::new(ScriptedFileDialogs::new()));
    app.enable_headless_instanced_scene();
    let started = Instant::now();
    assert!(app.open_document_path(&fixture));
    eprintln!(
        "exact-house open_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(app.document_path(), Some(fixture.as_path()));
    assert_eq!(app.document_snapshot().definitions().count(), BODIES);
    assert_eq!(app.document_snapshot().occurrences().count(), BODIES);
    assert!(!app.is_dirty());
    let digest = app.canonical_digest();
    let revision = app.document_revision();

    // Construct before worker connection so Harness's implicit startup settling
    // cannot spend an unbounded number of frames evaluating the exact house.
    let mut ui = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(
            |ctx, state: &mut BenchState| {
                let started = Instant::now();
                state.app.ui(ctx);
                state.ui_time += started.elapsed();
            },
            BenchState {
                app,
                ui_time: Duration::ZERO,
            },
        );
    ui.ctx.style_mut(|style| style.animation_time = 0.0);
    let exact_started = Instant::now();
    ui.state_mut().app.connect_exact_worker(&worker).unwrap();
    let mut polls = 0;
    while ui.state().app.exact_render_body_count() != BODIES {
        assert!(
            exact_started.elapsed() < DEADLINE,
            "exact house evaluation exceeded 180s: bodies={}/140 elapsed={:?}",
            ui.state().app.exact_render_body_count(),
            exact_started.elapsed(),
        );
        ui.step();
        polls += 1;
        if polls % 100 == 0 {
            eprintln!(
                "exact-house evaluating bodies={}/140 elapsed={:?}",
                ui.state().app.exact_render_body_count(),
                exact_started.elapsed()
            );
        }
        if ui.state().app.exact_render_body_count() != BODIES {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(
        exact_started.elapsed() < DEADLINE,
        "exact publication missed the 180s deadline"
    );
    let triangles = ui.state().app.exact_render_triangle_count();
    assert!(triangles > 0);
    eprintln!(
        "exact-house exact_ready_s={:.3} bodies={BODIES} triangles={triangles}",
        exact_started.elapsed().as_secs_f64()
    );
    ui.state_mut().app.zoom_fit();

    let probe = ketchup_interaction::Vec3::new(1000.0, 2000.0, 3000.0);
    let mut budgets = Vec::new();
    // Edges-off is diagnostic, not a workaround: default edges-on is guarded too.
    for edges in [true, false] {
        if !edges {
            ui.state_mut().app.toggle_edges();
        }
        for mode in ["shaded", "xray"] {
            if mode == "xray" {
                ui.state_mut().app.toggle_xray();
            }
            for _ in 0..3 {
                step(&mut ui);
            }
            if mode == "shaded" {
                assert!(ui.state().app.instanced_scene_triangle_count() > 0);
            }
            let idle: Vec<_> = (0..FRAMES).map(|_| step(&mut ui)).collect();
            budgets.push((mode, edges, "idle", report(mode, edges, "idle", &idle)));

            let mut pointer = ui.state().app.viewport_rect().unwrap().center();
            ui.input_mut().events.push(Event::PointerMoved(pointer));
            step(&mut ui);
            ui.input_mut().events.push(Event::PointerButton {
                pos: pointer,
                button: PointerButton::Secondary,
                pressed: true,
                modifiers: Modifiers::NONE,
            });
            step(&mut ui);
            // Warm the moving-camera path independently of the idle path.
            for _ in 0..3 {
                pointer += Vec2::new(3.0, -2.0);
                ui.input_mut().events.push(Event::PointerMoved(pointer));
                step(&mut ui);
            }
            let before = ui.state().app.viewport_position(probe).unwrap();
            let orbit: Vec<_> = (0..FRAMES)
                .map(|_| {
                    pointer += Vec2::new(3.0, -2.0);
                    ui.input_mut().events.push(Event::PointerMoved(pointer));
                    step(&mut ui)
                })
                .collect();
            let after = ui.state().app.viewport_position(probe).unwrap();
            budgets.push((mode, edges, "orbit", report(mode, edges, "orbit", &orbit)));
            assert!(
                before.distance(after) > 1.0,
                "{mode}: orbit must move the camera"
            );
            // Reverse the same drag, outside measurements, to compare every mode
            // at the same camera and zoom rather than progressively rotating away.
            pointer -= Vec2::new(3.0, -2.0) * 23.0;
            ui.input_mut().events.push(Event::PointerMoved(pointer));
            step(&mut ui);
            ui.input_mut().events.push(Event::PointerButton {
                pos: pointer,
                button: PointerButton::Secondary,
                pressed: false,
                modifiers: Modifiers::NONE,
            });
            step(&mut ui);
            assert_eq!(ui.state().app.exact_render_body_count(), BODIES);
            assert_eq!(ui.state().app.exact_render_triangle_count(), triangles);
            assert_eq!(ui.state().app.canonical_digest(), digest);
            assert_eq!(ui.state().app.document_revision(), revision);
            assert!(!ui.state().app.is_dirty());
            if mode == "xray" {
                ui.state_mut().app.toggle_xray();
            }
        }
    }
    ui.state_mut().app.toggle_edges();
    step(&mut ui);
    assert_eq!(ui.state().app.canonical_digest(), digest);
    assert_eq!(ui.state().app.document_revision(), revision);
    assert_eq!(ui.state().app.exact_render_body_count(), BODIES);
    assert!(!ui.state().app.is_dirty());
    assert_eq!(
        std::fs::read(&fixture).unwrap(),
        original_bytes,
        "fixture must remain untouched"
    );
    eprintln!("exact-house invariants=passed bodies=140 document=unchanged fixture=unchanged");
    // Report all eight series before failing; never hide a regression by relaxing
    // the budget. Includes AccessKit/harness preparation and egui tessellation,
    // excludes GPU callbacks and harness-only shape cloning/inspection.
    let violations: Vec<_> = budgets
        .iter()
        .filter(|(_, _, _, p95)| *p95 >= FRAME_BUDGET)
        .collect();
    assert!(
        violations.is_empty(),
        "100ms CPU-frame p95 budget exceeded: {violations:?}"
    );
}
