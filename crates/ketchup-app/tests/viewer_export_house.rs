//! Timing and content check of the 1 854-part house export, nightly in release.
//! cargo test -p ketchup-app --release --test integration viewer_export_house -- --ignored --nocapture

use crate::harness::poll_until;
use eframe::egui::{Pos2, Rect, Vec2};
use egui_kittest::kittest::Queryable as _;
use ketchup_app::{KetchupApp, dialogs::ScriptedFileDialogs};
use ketchup_view_format::Package;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(600);
const EXPORT_BUDGET: Duration = Duration::from_secs(2);

#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_house_viewer_export_finishes_within_seconds() {
    let _turn = crate::integration_support::file_turn();
    // A copy: the app writes work recovery next to the open document.
    let directory = tempfile::tempdir().expect("temp directory");
    let fixture = directory.path().join("house-project.ketchup");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-project.ketchup"),
        &fixture,
    )
    .expect("house example is required");
    let worker = PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"));
    let export_path = directory.path().join("menu.ketchup-view");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        ScriptedFileDialogs::new()
            .queue_open(&fixture)
            .queue_export(&export_path)
            .always_confirm_high_risk_as(84),
    ));
    app.enable_headless_instanced_scene();
    let mut ui = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app);
    let open_started = Instant::now();
    ui.input_mut().events.push(eframe::egui::Event::Key {
        key: eframe::egui::Key::O,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: eframe::egui::Modifiers::COMMAND,
    });
    let mut longest_open_frame = Duration::ZERO;
    let opened = poll_until(DEADLINE, || {
        let frame = Instant::now();
        ui.step();
        longest_open_frame = longest_open_frame.max(frame.elapsed());
        ui.state().document_snapshot().occurrences().count() == 1854
    });
    assert!(opened, "house never opened");
    eprintln!(
        "viewer-house open_ms={} longest_open_frame_ms={}",
        open_started.elapsed().as_millis(),
        longest_open_frame.as_millis()
    );
    assert!(
        longest_open_frame < EXPORT_BUDGET,
        "opening blocked a UI frame for {longest_open_frame:?}"
    );
    ui.state_mut()
        .connect_exact_worker(&worker)
        .expect("exact worker");
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
    let scene = ui
        .state_mut()
        .save_view("Viewer timing")
        .expect("save view");
    let started = Instant::now();
    let mut last_bodies = usize::MAX;
    let mut progress_shown = false;
    let mut exported = None;
    let ready = poll_until(DEADLINE, || {
        ui.step();
        if let Some((_, total)) = ui.state().exact_geometry_progress()
            && !progress_shown
        {
            ui.step();
            progress_shown = ui
                .query_all(egui_kittest::kittest::by().label_contains(&format!("/ {total}")))
                .next()
                .is_some();
        }
        let bodies = ui.state().exact_render_body_count();
        if bodies != last_bodies {
            eprintln!(
                "viewer-house bodies={bodies} elapsed={:?}",
                started.elapsed()
            );
            last_bodies = bodies;
            return false;
        }
        let attempt = Instant::now();
        match ui.state().current_model_viewer_export(&[scene], viewport) {
            Ok(export) => {
                exported = Some((export, attempt.elapsed()));
                true
            }
            Err(error) => {
                assert!(
                    attempt.elapsed() < EXPORT_BUDGET,
                    "a refused export attempt took {:?}: {error}",
                    attempt.elapsed()
                );
                false
            }
        }
    });
    assert!(ready, "exact house never became exportable");
    let (export, build) = exported.expect("export");
    let write = Instant::now();
    let mut bytes = Vec::new();
    export.package.write(&mut bytes).expect("write package");
    let write = write.elapsed();
    eprintln!(
        "viewer-house export build_ms={} write_ms={} bytes={} occurrences={}",
        build.as_millis(),
        write.as_millis(),
        bytes.len(),
        export.package.manifest.occurrences.len()
    );
    assert!(
        build + write < EXPORT_BUDGET,
        "export took {:?}",
        build + write
    );
    assert!(
        progress_shown,
        "loading the house showed computing progress"
    );
    let reopened = Package::read(bytes.as_slice()).expect("reopen exported package");
    assert_eq!(reopened, export.package);
    assert_eq!(reopened.manifest.start_scene, Some(scene.0));
    assert_eq!(reopened.manifest.scenes.len(), 1);
    assert_eq!(reopened.manifest.scenes[0].name, "Viewer timing");
    assert_house_contents(ui.state(), &reopened);
    perf_menu_export_and_save_within_budget(&mut ui, &export_path, &fixture);
}

fn perf_menu_export_and_save_within_budget(
    ui: &mut egui_kittest::Harness<'_, KetchupApp>,
    export_path: &std::path::Path,
    fixture: &std::path::Path,
) {
    ui.run();
    let file = ui.state().catalog().text("menu-file");
    ui.get_by_role_and_label(eframe::egui::accesskit::Role::Button, &file)
        .click();
    ui.run();
    let export = ui
        .state()
        .command_label(ketchup_app::AppCommand::ExportViewer);
    ui.get_by_role_and_label(eframe::egui::accesskit::Role::Button, &export)
        .click();
    let indication = format!("{}…", export.trim_end_matches(['.', '…']));
    let started = Instant::now();
    ui.step();
    assert!(
        ui.query_all(egui_kittest::kittest::by().label(&indication))
            .next()
            .is_some()
    );
    let mut longest_frame = started.elapsed();
    let finished = poll_until(Duration::from_secs(10), || {
        let frame = Instant::now();
        ui.step();
        longest_frame = longest_frame.max(frame.elapsed());
        ui.query_all(egui_kittest::kittest::by().label(&indication))
            .next()
            .is_none()
    });
    assert!(finished, "menu export never finished");
    let elapsed = started.elapsed();
    eprintln!(
        "viewer-house menu_export_ms={} longest_export_frame_ms={}",
        elapsed.as_millis(),
        longest_frame.as_millis()
    );
    assert!(elapsed < EXPORT_BUDGET, "menu export took {elapsed:?}");
    assert!(
        longest_frame < Duration::from_millis(500),
        "export blocked UI for {longest_frame:?}"
    );
    let package =
        Package::read(std::fs::File::open(export_path).expect("menu exported package")).unwrap();
    assert_house_contents(ui.state(), &package);
    assert!(
        std::fs::read_to_string(export_path.with_extension("ketchup-view.loss.txt"))
            .unwrap()
            .contains("including hidden components")
    );

    assert!(ui.state().is_dirty(), "saved reference view needs saving");
    ui.input_mut().events.push(eframe::egui::Event::Key {
        key: eframe::egui::Key::S,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: eframe::egui::Modifiers::COMMAND,
    });
    let started = Instant::now();
    let mut longest_frame = Duration::ZERO;
    let saved = poll_until(Duration::from_secs(30), || {
        let frame = Instant::now();
        ui.step();
        longest_frame = longest_frame.max(frame.elapsed());
        !ui.state().is_dirty()
    });
    assert!(saved, "house never saved");
    eprintln!(
        "viewer-house save_ms={} longest_save_frame_ms={}",
        started.elapsed().as_millis(),
        longest_frame.as_millis()
    );
    assert!(
        longest_frame < Duration::from_millis(500),
        "save blocked UI for {longest_frame:?}"
    );
    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_path(fixture));
    assert_eq!(reopened.document_snapshot().occurrences().count(), 1854);
    assert_eq!(reopened.saved_views(), ui.state().saved_views());
}

fn assert_house_contents(app: &KetchupApp, package: &Package) {
    let snapshot = app.document_snapshot();
    let source = snapshot.scene_query();
    assert_eq!(
        source.len(),
        1854,
        "reference fixture must contain the whole house"
    );
    assert_eq!(package.manifest.occurrences.len(), source.len());
    let geometry = package.geometry().expect("read renderable geometry");
    assert_eq!(
        geometry
            .nodes
            .iter()
            .filter(|node| node.mesh.is_some())
            .count(),
        1854
    );
    for part in &package.manifest.occurrences {
        assert!(
            part.path.steps.is_empty(),
            "this reference house has root parts"
        );
        let original = source
            .iter()
            .find(|item| item.instance_path.root_occurrence().0 == part.path.root_occurrence_id)
            .expect("exported part exists in source");
        assert_eq!(part.name, original.occurrence_name);
        assert_eq!(part.definition_id, original.definition_id.0);
        let body = geometry
            .traversal
            .iter()
            .copied()
            .find(|&index| {
                (index == part.glb_node || geometry.nodes[index].parent == Some(part.glb_node))
                    && geometry.nodes[index].mesh.is_some()
            })
            .expect("every part has renderable geometry");
        let node = &geometry.nodes[body];
        let mesh = node.mesh.expect("body mesh");
        assert!(
            geometry.meshes[mesh].iter().any(|primitive| {
                primitive.topology == ketchup_view_format::PrimitiveTopology::Triangles
                    && primitive.indices().len() > 0
            }),
            "part {} has triangles",
            part.name
        );
        let matrix = node.world_matrix;
        let actual_mm = [
            matrix[12] * 1000.0,
            -matrix[14] * 1000.0,
            matrix[13] * 1000.0,
        ];
        let expected_mm = original.transform.transform_point([0.0; 3]);
        for (actual, expected) in actual_mm.into_iter().zip(expected_mm) {
            assert!(
                (actual - expected).abs() < 1e-5,
                "part {} placement",
                part.name
            );
        }
    }
}
