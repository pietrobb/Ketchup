use super::*;
use crate::dialogs::ScriptedFileDialogs;
use crate::tests::shell::step_until;
use egui_kittest::{Harness, kittest::Queryable as _};

fn shell(dialogs: ScriptedFileDialogs) -> Harness<'static, KetchupApp> {
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.save_view("Shared").unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app)
}

fn wait(harness: &mut Harness<'_, KetchupApp>) {
    step_until(
        harness,
        Duration::from_secs(10),
        "export did not finish",
        |app| !app.file.activity.viewer_export.is_some(),
    );
}

#[test]
fn slow_preparation_is_visible_and_cancel_or_native_close_writes_nothing() {
    for native_close in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cancel.ketchup-view");
        let mut harness = shell(ScriptedFileDialogs::new().always_confirm_high_risk_as(84));
        let input = harness.state().viewer_file_input(&path).unwrap();
        let (release, gate) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let context = harness.ctx.clone();
        harness.state_mut().queue_viewer_export(
            path.clone(),
            input,
            &context,
            move |path, input| {
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
                let result = PreparedExport::prepare(path, input);
                finished.send(()).unwrap();
                result
            },
        );
        // The worker waits for `release`, sent below: a frame that waited for
        // it would end only once the worker gave up, with no export to show.
        for _ in 0..8 {
            harness.step();
            let label = harness.state().file_activity_label().unwrap();
            assert!(
                harness
                    .query_all(egui_kittest::kittest::by().label(&label))
                    .next()
                    .is_some()
            );
        }
        if native_close {
            harness
                .input_mut()
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        } else {
            harness.input_mut().events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        harness.step();
        assert!(harness.state().file.activity.viewer_export.is_none());
        assert_eq!(harness.state().close_guard.pending, native_close);
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(10)).unwrap();
        harness.step();
        assert!(!path.exists());
        assert!(!path.with_extension("ketchup-view.loss.txt").exists());
        assert!(harness.state().last_side_effect_receipt().is_none());
    }
}

#[test]
fn capture_defers_geometry_and_worker_uses_the_captured_document() {
    let path = Path::new("captured.ketchup-view");
    let mut harness = shell(ScriptedFileDialogs::new());
    let input = harness.state().viewer_file_input(path).unwrap();
    let expected = harness
        .state()
        .current_model_viewer_export(
            &harness.state().viewer_export_scenes(&input.settings),
            harness.state().camera.viewport_rect.unwrap(),
        )
        .unwrap()
        .package;
    harness.state_mut().exact.results = ExactResultRegistry::default();
    // Capturing inputs succeeds without renderable geometry; only the worker builds it.
    let unavailable = harness.state().viewer_file_input(path).unwrap();
    assert!(
        std::thread::spawn(move || unavailable.build())
            .join()
            .unwrap()
            .is_err()
    );
    assert!(harness.state_mut().create_box());
    let exported = std::thread::spawn(move || input.build())
        .join()
        .unwrap()
        .unwrap();
    assert_eq!(exported.package, expected);
    assert_eq!(exported.package.manifest.occurrences.len(), 1);
}

#[test]
fn pure_package_preparation_does_not_inspect_or_recover_the_destination() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("destination.ketchup-view");
    // An in-progress transaction may temporarily have an unusable destination.
    // Pure preparation must not inspect it, let alone repair another export.
    std::fs::create_dir(&path).unwrap();
    let report = path.with_extension("ketchup-view.loss.txt");
    std::fs::write(&report, b"another writer's report").unwrap();
    let harness = shell(ScriptedFileDialogs::new());
    let input = harness.state().viewer_file_input(&path).unwrap();
    let prepared = PreparedExport::prepare(&path, input)
        .expect("preparation is independent of destination I/O");
    let package = ketchup_view_format::Package::read(prepared.bytes.as_slice()).unwrap();
    assert_eq!(package.manifest.occurrences.len(), 1);
    assert!(path.is_dir());
    assert_eq!(std::fs::read(&report).unwrap(), b"another writer's report");
}

#[test]
fn publication_wait_is_responsive_and_close_is_sent_only_after_success() {
    for fail in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("publish.ketchup-view");
        let mut harness = shell(ScriptedFileDialogs::new().always_confirm_high_risk_as(84));
        let input = harness.state().viewer_file_input(&path).unwrap();
        let document = input.snapshot.document_id();
        let revision = input.snapshot.revision_id();
        let settings = input.settings.clone();
        let prepared = PreparedExport::prepare(&path, input).unwrap();
        let precondition =
            ExportBundlePrecondition::capture(&path, &path.with_extension("ketchup-view.loss.txt"))
                .unwrap();
        prepared
            .authorize(harness.state_mut(), &path, &precondition)
            .unwrap();
        let (release, gate) = mpsc::channel();
        let (sender, receiver) = mpsc::channel();
        let worker_path = path.clone();
        std::thread::spawn(move || {
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            sender
                .send(prepared.write(&worker_path, &precondition))
                .unwrap();
        });
        harness.state_mut().file.activity.viewer_export = Some(ViewerExportTask {
            path: path.clone(),
            document,
            revision,
            settings,
            stage: Stage::Writing(receiver),
            close_requested: false,
        });
        // The writer waits for `release`: a frame that waited for it would end
        // only once the writer gave up, with no write to show.
        for _ in 0..8 {
            harness.step();
            let label = harness.state().file_activity_label().unwrap();
            assert!(
                harness
                    .query_all(egui_kittest::kittest::by().label(&label))
                    .next()
                    .is_some()
            );
        }
        harness
            .input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        harness.step();
        assert!(
            harness.output().viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::CancelClose)
        );
        harness
            .input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .clear();
        if fail {
            std::fs::create_dir(&path).unwrap();
        }
        release.send(()).unwrap();
        wait(&mut harness);
        assert_eq!(
            harness.output().viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::Close),
            !fail
        );
        if fail {
            assert!(path.is_dir());
            assert!(harness.state().digest.contains("publish.ketchup-view"));
        } else {
            let package =
                ketchup_view_format::Package::read(std::fs::File::open(&path).unwrap()).unwrap();
            assert_eq!(package.manifest.occurrences.len(), 1);
            assert!(path.with_extension("ketchup-view.loss.txt").is_file());
        }
    }
}

#[test]
fn changed_sharing_or_geometry_during_preparation_never_publishes_old_content() {
    for sharing_changed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("stale.ketchup-view");
        let mut harness = shell(ScriptedFileDialogs::new().always_confirm_high_risk_as(84));
        let input = harness.state().viewer_file_input(&path).unwrap();
        let (release, gate) = mpsc::channel();
        let context = harness.ctx.clone();
        harness.state_mut().queue_viewer_export(
            path.clone(),
            input,
            &context,
            move |path, input| {
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
                PreparedExport::prepare(path, input)
            },
        );
        if sharing_changed {
            let mut settings = harness.state().stored_viewer_settings().unwrap();
            settings.excluded_roots.insert(1);
            assert!(harness.state_mut().store_viewer_settings(settings));
        } else {
            assert!(harness.state_mut().create_box());
        }
        release.send(()).unwrap();
        wait(&mut harness);
        assert!(!path.exists());
        assert!(!path.with_extension("ketchup-view.loss.txt").exists());
        assert!(harness.state().last_side_effect_receipt().is_none());
    }
}

#[test]
fn background_export_preserves_unapproved_targets_and_reports_publication_failure() {
    for missing_directory in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = if missing_directory {
            directory.path().join("missing").join("model.ketchup-view")
        } else {
            directory.path().join("model.ketchup-view")
        };
        let report = path.with_extension("ketchup-view.loss.txt");
        let dialogs = if missing_directory {
            ScriptedFileDialogs::new().always_confirm_high_risk_as(84)
        } else {
            ScriptedFileDialogs::new()
        };
        if !missing_directory {
            std::fs::write(&path, b"original package").unwrap();
            std::fs::write(&report, b"original report").unwrap();
        }
        let mut harness = shell(dialogs);
        let context = harness.ctx.clone();
        harness
            .state_mut()
            .start_viewer_export(path.clone(), &context);
        wait(&mut harness);
        if missing_directory {
            assert!(!path.exists());
            assert!(!report.exists());
            assert!(harness.state().digest.contains("model.ketchup-view"));
        } else {
            assert_eq!(std::fs::read(&path).unwrap(), b"original package");
            assert_eq!(std::fs::read(&report).unwrap(), b"original report");
        }
        assert!(
            !harness.output().viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::Close)
        );
    }
}
