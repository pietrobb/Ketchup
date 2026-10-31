//! Exports the Viewer reference packages measured in docs/viewer/performance.md:
//! the nested furniture set and the instanced house, as File > Export for Viewer
//! does. The HPL kitchen is the committed viewer/samples package. The nightly
//! build checks that both stay exportable; with KETCHUP_TEST_REPORT_DIR set the
//! packages are written there for measuring. The programs of the examples run in
//! ketchup-program-worker beside the exact worker:
//! cargo build --release --locked -p ketchup-scheduler --bin ketchup-program-worker
//! set KETCHUP_TEST_REPORT_DIR=C:\some\dir
//! cargo test -p ketchup-app --release --test integration viewer_reference_packages -- --ignored --nocapture

use crate::harness::poll_until;
use eframe::egui::{Pos2, Rect, Vec2};
use ketchup_app::KetchupApp;
use ketchup_view_format::Package;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(180);

#[test]
#[ignore = "exports the whole house; release build, nightly"]
fn export_viewer_reference_packages() {
    let _turn = crate::integration_support::file_turn();
    for example in ["house-project", "wardrobe-chair-table"] {
        export_example(example);
    }
}

fn export_example(example: &str) {
    // A copy: the app writes work recovery next to the open document.
    let directory = tempfile::tempdir().expect("temp directory");
    let fixture = directory.path().join(format!("{example}.ketchup"));
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(format!("{example}.ketchup")),
        &fixture,
    )
    .expect("reference example");
    let mut app = KetchupApp::new();
    app.enable_headless_instanced_scene();
    assert!(app.open_document_path(&fixture), "{example} did not open");
    let mut ui = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app);
    ui.state_mut()
        .connect_exact_worker(Path::new(env!(
            "CARGO_BIN_EXE_ketchup-performance-exact-worker"
        )))
        .expect("exact worker");
    let scene = ui.state_mut().save_view("Reference").expect("save view");
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
    let started = Instant::now();
    let mut last_bodies = usize::MAX;
    let mut exported = None;
    let mut refused = None;
    let ready = poll_until(DEADLINE, || {
        ui.step();
        let bodies = ui.state().exact_render_body_count();
        if bodies != last_bodies {
            last_bodies = bodies;
            return false;
        }
        match ui.state().current_model_viewer_export(&[scene], viewport) {
            Ok(export) => exported = Some(export),
            Err(error) => refused = Some(format!("{error:?}")),
        }
        exported.is_some()
    });
    assert!(
        ready,
        "{example} never became exportable with {last_bodies} bodies: {refused:?}"
    );
    let export = exported.expect("export");
    let mut bytes = Vec::new();
    export.package.write(&mut bytes).expect("write package");
    assert_eq!(
        Package::read(bytes.as_slice()).expect("reopen"),
        export.package
    );
    assert!(
        !export.package.manifest.occurrences.is_empty(),
        "{example} exported no parts"
    );
    let path = ketchup_test_env::report_path(&format!("{example}.ketchup-view"));
    if let Some(path) = &path {
        std::fs::write(path, &bytes).expect("write reference package");
    }
    eprintln!(
        "{example}: {} bytes, {} parts, {} bodies, ready after {:?} -> {:?}",
        bytes.len(),
        export.package.manifest.occurrences.len(),
        last_bodies,
        started.elapsed(),
        path
    );
}
