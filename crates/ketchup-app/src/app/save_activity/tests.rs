use super::*;
use crate::dialogs::ScriptedFileDialogs;
use crate::tests::shell::step_until;
use egui_kittest::{Harness, kittest::Queryable as _};

fn shell(dialogs: ScriptedFileDialogs) -> Harness<'static, KetchupApp> {
    let app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app)
}

fn wait(harness: &mut Harness<'_, KetchupApp>) {
    step_until(
        harness,
        Duration::from_secs(10),
        "save did not finish",
        |app| !app.file.activity.saving.is_some(),
    );
}

fn assert_indicated(harness: &mut Harness<'_, KetchupApp>) {
    for _ in 0..8 {
        let started = Instant::now();
        harness.step();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "UI waited for file worker"
        );
        let label = harness
            .state()
            .file_activity_label()
            .expect("save indication");
        assert!(
            harness
                .query_all(egui_kittest::kittest::by().label(&label))
                .next()
                .is_some()
        );
        assert!(harness.state().is_dirty());
    }
}

fn close(harness: &mut Harness<'_, KetchupApp>) {
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    harness.step();
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .clear();
}

#[test]
fn preparation_is_responsive_and_cancel_or_close_never_writes() {
    let directory = tempfile::tempdir().unwrap();
    for native_close in [false, true] {
        let path = directory
            .path()
            .join(format!("cancel-{native_close}.ketchup"));
        let mut harness = shell(ScriptedFileDialogs::new());
        assert!(harness.state_mut().create_box());
        let (release, gate) = mpsc::channel();
        let (finished, completion) = mpsc::channel();
        let context = harness.ctx.clone();
        harness.state_mut().queue_save_preparation(
            path.clone(),
            &context,
            move |document, container| {
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
                let result = prepare_save(document, container);
                finished.send(()).unwrap();
                result
            },
        );
        assert_indicated(&mut harness);
        if native_close {
            close(&mut harness);
        } else {
            harness.input_mut().events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
        }
        assert!(harness.state().file.activity.saving.is_none());
        assert_eq!(harness.state().close_guard.pending, native_close);
        release.send(()).unwrap();
        completion.recv_timeout(Duration::from_secs(10)).unwrap();
        harness.step();
        assert!(!path.exists());
        assert!(harness.state().is_dirty());
    }
}

#[test]
fn actual_locked_write_keeps_ui_alive_then_reopens_saved_geometry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("locked.ketchup");
    let lock = std::fs::File::create(path.with_extension("ketchup.save-lock")).unwrap();
    lock.lock().unwrap();
    let mut harness = shell(ScriptedFileDialogs::new());
    assert!(harness.state_mut().create_box());
    let expected_count = harness.state().document.current().occurrences().count();
    let context = harness.ctx.clone();
    harness
        .state_mut()
        .start_background_save(path.clone(), &context);
    step_until(
        &mut harness,
        Duration::from_secs(10),
        "save did not start writing",
        |app| app.native_save_is_writing(),
    );
    assert_indicated(&mut harness);
    assert!(!path.exists());
    close(&mut harness);
    assert!(
        harness.output().viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose)
    );
    assert!(harness.state().native_save_is_writing());
    lock.unlock().unwrap();
    wait(&mut harness);
    assert!(!harness.state().is_dirty());
    assert!(
        harness.output().viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Close)
    );
    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_path(&path));
    assert_eq!(
        reopened.document.current().occurrences().count(),
        expected_count
    );
}

#[test]
fn an_edit_during_write_stays_dirty_and_is_not_in_the_saved_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("edited.ketchup");
    let lock = std::fs::File::create(path.with_extension("ketchup.save-lock")).unwrap();
    lock.lock().unwrap();
    let mut harness = shell(ScriptedFileDialogs::new());
    let before = harness.state().document.current().occurrences().count();
    let context = harness.ctx.clone();
    harness
        .state_mut()
        .start_background_save(path.clone(), &context);
    step_until(
        &mut harness,
        Duration::from_secs(10),
        "save did not start writing",
        |app| app.native_save_is_writing(),
    );
    assert!(harness.state_mut().create_box());
    lock.unlock().unwrap();
    wait(&mut harness);
    assert!(harness.state().is_dirty());
    assert!(harness.state().document.current().occurrences().count() > before);
    let persistence::LoadOutcome::Editable { document, .. } =
        persistence::load(&std::fs::read(&path).unwrap()).unwrap()
    else {
        panic!("saved file is not editable");
    };
    assert_eq!(document.current().occurrences().count(), before);
}

#[test]
fn background_save_refuses_unapproved_overwrite_and_preserves_existing_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("protected.ketchup");
    assert!(KetchupApp::new().save_document_to(&path));
    let original = std::fs::read(&path).unwrap();
    let dialogs = ScriptedFileDialogs::new();
    let script = dialogs.clone();
    let mut harness = shell(dialogs);
    assert!(harness.state_mut().create_box());
    let context = harness.ctx.clone();
    harness
        .state_mut()
        .start_background_save(path.clone(), &context);
    wait(&mut harness);
    assert!(!script.high_risk_prompts().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(harness.state().is_dirty());
}

#[test]
fn a_destination_conflict_is_reported_even_after_an_edit_during_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conflict.ketchup");
    let lock = std::fs::File::create(path.with_extension("ketchup.save-lock")).unwrap();
    lock.lock().unwrap();
    let mut harness = shell(ScriptedFileDialogs::new());
    assert!(harness.state_mut().create_box());
    let context = harness.ctx.clone();
    harness
        .state_mut()
        .start_background_save(path.clone(), &context);
    step_until(
        &mut harness,
        Duration::from_secs(10),
        "save did not start writing",
        |app| app.native_save_is_writing(),
    );
    assert!(harness.state_mut().create_box());
    close(&mut harness);
    let foreign = b"another writer owns this destination";
    std::fs::write(&path, foreign).unwrap();
    lock.unlock().unwrap();
    wait(&mut harness);
    assert_eq!(std::fs::read(&path).unwrap(), foreign);
    assert!(harness.state().is_dirty());
    assert!(harness.state().file.path.is_none());
    assert!(harness.state().digest.contains("conflict.ketchup"));
    assert!(
        !harness.output().viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Close)
    );
}

#[test]
fn failed_save_of_recovered_work_retains_exclusive_recovery_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recovered.ketchup");
    let mut original = KetchupApp::new();
    assert!(original.save_document_to(&path));
    let primary = std::fs::read(&path).unwrap();
    assert!(original.create_box());
    original.refresh_work_recovery_checkpoint();
    assert!(original.file.work_recovery_identity.is_some());
    drop(original);
    let mut harness = shell(ScriptedFileDialogs::new().always_confirm_high_risk_as(84));
    assert!(harness.state_mut().open_document_path(&path));
    assert!(harness.state().file.path.is_none());
    assert!(harness.state().file.recovery_open.is_some());
    assert!(WorkRecoveryLock::acquire(&path).is_err());
    std::fs::create_dir(path.with_extension("ketchup.recovery")).unwrap();
    let context = harness.ctx.clone();
    harness
        .state_mut()
        .start_background_save(path.clone(), &context);
    wait(&mut harness);
    assert!(harness.state().is_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), primary);
    assert!(
        WorkRecoveryLock::acquire(&path).is_err(),
        "failed save released the active recovery owner"
    );
}

#[test]
fn an_edit_during_preparation_discards_the_outdated_save_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("outdated.ketchup");
    let mut harness = shell(ScriptedFileDialogs::new());
    let context = harness.ctx.clone();
    let (release, gate) = mpsc::channel();
    harness.state_mut().queue_save_preparation(
        path.clone(),
        &context,
        move |document, container| {
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            prepare_save(document, container)
        },
    );
    assert!(harness.state_mut().create_box());
    release.send(()).unwrap();
    wait(&mut harness);
    assert!(!path.exists());
    assert!(harness.state().is_dirty());
}
