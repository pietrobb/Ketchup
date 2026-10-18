//! Opt-in release workflow on real documents, using only headless UI input.
//! KETCHUP_RELEASE_FIXTURE names the kitchen document; artifacts go to a new
//! `viewer-release` directory under KETCHUP_TEST_REPORT_DIR.
use crate::harness::poll_until;
use eframe::egui::{self, Vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use ketchup_app::{KetchupApp, dialogs::ScriptedFileDialogs};
use ketchup_viewer::{app::ViewerApp, camera::Camera, model::Model, text::Language};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "native worker, GPU, real kitchen fixture and explicit artifact destination required"]
fn perf_house_and_kitchen_open_export_and_render_in_the_viewer() {
    let _turn = crate::integration_support::file_turn();
    let kitchen = ketchup_test_env::release_fixture().expect("kitchen fixture path");
    let output = ketchup_test_env::report_path("viewer-release").expect("report directory");
    std::fs::create_dir(&output).expect("artifact directory must be new");
    for (name, source) in [
        (
            "house",
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/house-project.ketchup"),
        ),
        ("kitchen", kitchen),
    ] {
        let package = perf_export_copy(&source, &output, name);
        render_viewer(&package, &output, name);
    }
}

fn perf_export_copy(source: &Path, output: &Path, name: &str) -> std::path::PathBuf {
    let temporary = tempfile::tempdir().expect("isolated source copy");
    let source_copy = temporary.path().join("source.ketchup");
    std::fs::copy(source, &source_copy).expect("copy source, never write its recovery");
    let started = Instant::now();
    let path = output.join(format!("{name}.ketchup-view"));
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        ScriptedFileDialogs::new()
            .queue_open(&source_copy)
            .queue_export(&path)
            .always_confirm_high_risk_as(84),
    ));
    app.enable_headless_instanced_scene();
    let mut ui = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .renderer(ketchup_test_gpu::renderer())
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app);
    ui.input_mut().events.push(egui::Event::Key {
        key: egui::Key::O,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    let mut longest_open_frame = Duration::ZERO;
    let opened = poll_until(Duration::from_secs(30), || {
        let frame = Instant::now();
        ui.step();
        longest_open_frame = longest_open_frame.max(frame.elapsed());
        ui.state().document_path() == Some(source_copy.as_path())
    });
    assert!(opened, "{name} did not open");
    let open = started.elapsed();
    assert!(longest_open_frame < Duration::from_millis(500));
    eprintln!(
        "VQ {name}: longest_open_frame_ms={}",
        longest_open_frame.as_millis()
    );
    ui.state_mut()
        .connect_exact_worker(Path::new(env!(
            "CARGO_BIN_EXE_ketchup-performance-exact-worker"
        )))
        .expect("native worker");
    let mut scenes: Vec<_> = ui.state().saved_views().iter().map(|(id, _)| *id).collect();
    if scenes.is_empty() {
        ui.state_mut().zoom_fit();
        scenes.push(
            ui.state_mut()
                .save_view("Release reference")
                .expect("reference scene"),
        );
    }
    for &scene in &scenes {
        assert!(ui.state_mut().set_viewer_scene_shared(scene, true));
    }
    assert!(ui.state_mut().set_viewer_start_scene(scenes[0]));
    let viewport = ui.state().viewport_rect().expect("laid out viewport");
    let mut exported = None;
    let ready = poll_until(Duration::from_secs(600), || {
        ui.step();
        let attempt = Instant::now();
        let export = ui
            .state()
            .current_model_viewer_export(&scenes, viewport)
            .ok();
        exported = export.map(|export| (export, attempt.elapsed()));
        exported.is_some()
    });
    assert!(ready, "{name} never became exportable");
    let (export, build) = exported.expect("export");
    ui.run();
    let menu = ui.state().catalog().text("menu-file");
    ui.get_by_role_and_label(egui::accesskit::Role::Button, &menu)
        .click();
    ui.run();
    let command = ui
        .state()
        .command_label(ketchup_app::AppCommand::ExportViewer);
    ui.get_by_role_and_label(egui::accesskit::Role::Button, &command)
        .click();
    let indication = format!("{}…", command.trim_end_matches(['.', '…']));
    let write = Instant::now();
    ui.step();
    let mut longest_export_frame = write.elapsed();
    assert!(
        ui.query_all(egui_kittest::kittest::by().label(&indication))
            .next()
            .is_some()
    );
    let finished = poll_until(Duration::from_secs(10), || {
        let frame = Instant::now();
        ui.step();
        longest_export_frame = longest_export_frame.max(frame.elapsed());
        ui.query_all(egui_kittest::kittest::by().label(&indication))
            .next()
            .is_none()
    });
    assert!(finished, "{name} did not export");
    let write = write.elapsed();
    assert!(
        write < Duration::from_secs(2),
        "{name} menu export took {write:?}"
    );
    assert!(longest_export_frame < Duration::from_millis(500));
    eprintln!(
        "VQ {name}: longest_export_frame_ms={}",
        longest_export_frame.as_millis()
    );
    assert!(path.with_extension("ketchup-view.loss.txt").is_file());
    let reopened = ketchup_view_format::Package::read(std::fs::File::open(&path).unwrap())
        .expect("reopen export");
    for (a, b) in reopened
        .manifest
        .definitions
        .iter()
        .zip(&export.package.manifest.definitions)
    {
        assert_eq!(a, b, "definition roundtrip");
    }
    for (a, b) in reopened
        .manifest
        .occurrences
        .iter()
        .zip(&export.package.manifest.occurrences)
    {
        assert_eq!(a, b, "occurrence roundtrip");
    }
    assert_eq!(
        reopened, export.package,
        "all package fields roundtrip exactly"
    );
    assert_eq!(reopened.manifest.scenes.len(), scenes.len());
    assert!(!reopened.manifest.occurrences.is_empty());
    if name == "house" {
        assert_eq!(reopened.manifest.occurrences.len(), 1854);
    }
    eprintln!(
        "VQ {name}: open_ms={} ready_ms={} build_ms={} menu_export_ms={} parts={} scenes={}",
        open.as_millis(),
        started.elapsed().as_millis(),
        build.as_millis(),
        write.as_millis(),
        reopened.manifest.occurrences.len(),
        reopened.manifest.scenes.len()
    );
    perf_save_and_reopen(&mut ui, &source_copy, name);
    path
}

fn perf_save_and_reopen(ui: &mut Harness<'_, KetchupApp>, path: &Path, name: &str) {
    assert!(ui.state().is_dirty());
    let expected_views = ui.state().saved_views();
    let expected_parts = ui.state().document_snapshot().occurrences().count();
    ui.input_mut().events.push(egui::Event::Key {
        key: egui::Key::S,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    let started = Instant::now();
    let mut longest_frame = Duration::ZERO;
    let saved = poll_until(Duration::from_secs(30), || {
        let frame = Instant::now();
        ui.step();
        longest_frame = longest_frame.max(frame.elapsed());
        !ui.state().is_dirty()
    });
    assert!(saved, "{name} did not save");
    eprintln!(
        "VQ {name}: save_ms={} longest_save_frame_ms={}",
        started.elapsed().as_millis(),
        longest_frame.as_millis()
    );
    assert!(longest_frame < Duration::from_millis(500));
    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_path(path));
    assert_eq!(reopened.saved_views(), expected_views);
    assert_eq!(
        reopened.document_snapshot().occurrences().count(),
        expected_parts
    );
}

fn render_viewer(path: &Path, output: &Path, name: &str) {
    let render_state = ketchup_test_gpu::render_state();
    eprintln!("VQ GPU: {:?}", render_state.adapter.get_info());
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(render_state.clone());
    let mut viewer =
        ViewerApp::new(Some(render_state)).with_language(Language::from_tag("en").unwrap());
    let opened = Instant::now();
    viewer.open_path(path).expect("open real export in Viewer");
    eprintln!("VQ {name}: viewer_open_ms={}", opened.elapsed().as_millis());
    let model = Model::read(std::fs::File::open(path).unwrap()).expect("reference model");
    let mut ui = Harness::builder()
        .with_size(Vec2::new(1200.0, 800.0))
        .renderer(renderer)
        .build_state(|ctx, viewer: &mut ViewerApp| viewer.show(ctx), viewer);
    ui.run();
    ui.render()
        .expect("render initial scene")
        .save(output.join(format!("{name}-initial.png")))
        .expect("initial image");
    for scene in &model.manifest().scenes {
        ui.get_by_role_and_label(egui::accesskit::Role::Button, &scene.name)
            .click();
        ui.run();
        ui.render()
            .expect("render saved scene")
            .save(output.join(format!("{name}-scene-{}.png", scene.id)))
            .expect("scene image");
    }
    ui.get_by_label("Fit").click();
    ui.run();
    let before = ui.render().expect("render fitted model");
    before
        .save(output.join(format!("{name}-fit.png")))
        .expect("fit image");
    let rect = ui.get_by_label("Model view").rect();
    let camera = Camera::fit(&model);
    // Fit preserves the last scene's visibility. Restore its actual hidden paths.
    let scene = model.manifest().scenes.last().expect("scene");
    let visible: Vec<_> = model
        .manifest()
        .occurrences
        .iter()
        .map(|part| {
            !scene.hidden.iter().any(|hidden| {
                part.path.root_occurrence_id == hidden.root_occurrence_id
                    && part.path.steps.starts_with(&hidden.steps)
            })
        })
        .collect();
    let mut hit = None;
    'scan: for y in [0.0, 0.2, -0.2, 0.4, -0.4, 0.6, -0.6] {
        for x in [0.0, 0.2, -0.2, 0.4, -0.4, 0.6, -0.6] {
            if let Some(index) = camera.pick(
                &model,
                [x, y],
                rect.width() / rect.height(),
                &visible,
                scene.section.as_ref(),
            ) && model.manifest().definitions.iter().any(|definition| {
                definition.id == model.manifest().occurrences[index].definition_id
                    && definition.local_dimensions.is_some()
            }) {
                hit = Some((index, [x, y]));
                break 'scan;
            }
        }
    }
    let (selected, ndc) = hit.expect("at least one dimensioned real part is visible and pickable");
    let point = egui::pos2(
        rect.left() + (ndc[0] + 1.0) * 0.5 * rect.width(),
        rect.top() + (1.0 - ndc[1]) * 0.5 * rect.height(),
    );
    ui.input_mut().events.extend([
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    ui.step();
    ui.input_mut().events.push(egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    ui.run();
    let part = &model.manifest().occurrences[selected];
    assert!(
        ui.query_all(egui_kittest::kittest::by().label(&part.name))
            .next()
            .is_some(),
        "picked part name shown"
    );
    assert!(
        ui.query_all(egui_kittest::kittest::by().label_contains("Size:"))
            .next()
            .is_some(),
        "picked part dimensions shown"
    );
    let after = ui.render().expect("render selection");
    after
        .save(output.join(format!("{name}-selected.png")))
        .expect("selection image");
    let mut changed = 0;
    let mut highlighted = 0;
    for y in rect.top() as u32..rect.bottom() as u32 {
        for x in rect.left() as u32..rect.right() as u32 {
            if before.get_pixel(x, y) != after.get_pixel(x, y) {
                changed += 1;
                let [r, g, b, _] = after.get_pixel(x, y).0;
                if r > 180 && g > 100 && r > b.saturating_add(50) {
                    highlighted += 1;
                }
            }
        }
    }
    assert!(
        highlighted > 10,
        "the selected part is visibly orange, not only a changed viewport"
    );
    assert!(
        changed > 10,
        "selection changes model pixels, not just the information panel"
    );
}
