use crate::tests::shell::step_until;
use crate::{KetchupApp, dialogs::ScriptedFileDialogs};
use eframe::egui::{self, ViewportCommand, ViewportEvent, ViewportId, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable as _};
use std::time::Duration;

fn shell(dialogs: ScriptedFileDialogs, dirty: bool) -> Harness<'static, KetchupApp> {
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    if dirty {
        assert!(app.create_box());
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_max_steps(64)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    harness
}

fn close_event(harness: &mut Harness<'_, KetchupApp>) {
    harness
        .input_mut()
        .viewports
        .entry(ViewportId::ROOT)
        .or_default()
        .events
        .push(ViewportEvent::Close);
    harness.step();
    // The headless runner has no native backend to consume viewport events.
    harness
        .input_mut()
        .viewports
        .entry(ViewportId::ROOT)
        .or_default()
        .events
        .clear();
}

fn canceled(harness: &Harness<'_, KetchupApp>) -> bool {
    harness.output().viewport_output[&ViewportId::ROOT]
        .commands
        .iter()
        .any(|command| matches!(command, ViewportCommand::CancelClose))
}

fn click(harness: &mut Harness<'_, KetchupApp>, key: &str) {
    harness.run();
    let label = harness.state().catalog.text(key);
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.step();
    step_until(
        harness,
        Duration::from_secs(10),
        "save did not finish",
        |app| !app.native_save_in_progress(),
    );
    assert_eq!(
        harness.output().viewport_output[&ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, ViewportCommand::Close)),
        harness.state().close_guard.approved.is_some()
    );
    harness.run();
}

#[test]
fn clean_native_close_does_not_prompt() {
    let mut harness = shell(ScriptedFileDialogs::new(), false);
    assert!(!harness.state().is_dirty());
    close_event(&mut harness);
    assert!(!canceled(&harness));
    assert!(!harness.state().close_guard.pending);
}

#[test]
fn dirty_native_close_is_canceled_and_cancel_preserves_history() {
    let mut harness = shell(ScriptedFileDialogs::new(), true);
    let before = harness.state().document.history_digest();
    close_event(&mut harness);
    assert!(canceled(&harness));
    assert!(harness.state().close_guard.pending);
    click(&mut harness, "action-cancel");
    assert!(!harness.state().close_guard.pending);
    assert!(harness.state().close_guard.approved.is_none());
    assert!(harness.state().is_dirty());
    assert_eq!(harness.state().document.history_digest(), before);
    close_event(&mut harness);
    assert!(canceled(&harness));
}

#[test]
fn discard_allows_one_close_without_mutating_or_marking_work_saved() {
    let mut harness = shell(ScriptedFileDialogs::new(), true);
    let before = harness.state().document.history_digest();
    close_event(&mut harness);
    assert!(canceled(&harness));
    click(&mut harness, "dialog-close-discard");
    assert!(harness.state().close_guard.approved.is_some());
    assert!(harness.state().is_dirty());
    close_event(&mut harness);
    assert!(!canceled(&harness));
    assert!(harness.state().close_guard.approved.is_none());
    assert_eq!(harness.state().document.history_digest(), before);
    close_event(&mut harness);
    assert!(canceled(&harness), "discard approval must be single-use");
}

#[test]
fn discard_does_not_authorize_later_model_changes() {
    let mut harness = shell(ScriptedFileDialogs::new(), true);
    close_event(&mut harness);
    click(&mut harness, "dialog-close-discard");
    assert!(harness.state_mut().create_box());
    close_event(&mut harness);
    assert!(canceled(&harness));
    assert!(harness.state().close_guard.pending);
    assert!(harness.state().close_guard.approved.is_none());
}

#[test]
fn save_on_close_preserves_model_and_undo_on_disk() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("close-save.ketchup");
    let mut harness = shell(ScriptedFileDialogs::new().queue_save(&path), true);
    let history = harness.state().document.history_digest();
    close_event(&mut harness);
    assert!(canceled(&harness));
    click(&mut harness, "file-save");
    assert!(!harness.state().is_dirty());
    assert!(path.is_file());
    assert!(harness.state().close_guard.approved.is_some());
    close_event(&mut harness);
    assert!(!canceled(&harness));
    let mut reopened = KetchupApp::new().with_dialogs(Box::new(ScriptedFileDialogs::new()));
    reopened.open_document_from(&path);
    assert_eq!(reopened.document.history_digest(), history);
    assert!(!reopened.is_dirty());
}

#[test]
fn canceled_save_and_failed_write_leave_the_document_open_and_dirty() {
    let directory = tempfile::tempdir().unwrap();
    let scripts = [
        ScriptedFileDialogs::new().queue_cancelled_save(),
        ScriptedFileDialogs::new().queue_save(directory.path()),
        ScriptedFileDialogs::new().queue_save(directory.path().join("missing").join("x.ketchup")),
    ];
    for script in scripts {
        let mut harness = shell(script, true);
        let history = harness.state().document.history_digest();
        close_event(&mut harness);
        click(&mut harness, "file-save");
        assert!(harness.state().is_dirty());
        assert!(harness.state().close_guard.pending);
        assert!(harness.state().close_guard.approved.is_none());
        assert_eq!(harness.state().document.history_digest(), history);
        close_event(&mut harness);
        assert!(canceled(&harness));
    }
}

#[test]
fn save_on_close_remains_responsive_while_disk_is_locked_and_protects_new_edits() {
    for concurrent_edit in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("close-locked.ketchup");
        let lock = std::fs::File::create(path.with_extension("ketchup.save-lock")).unwrap();
        lock.lock().unwrap();
        let mut harness = shell(ScriptedFileDialogs::new().queue_save(&path), true);
        close_event(&mut harness);
        harness.run();
        let save = harness.state().catalog.text("file-save");
        harness.get_by_role_and_label(Role::Button, &save).click();
        harness.step();
        step_until(
            &mut harness,
            Duration::from_secs(10),
            "save did not start writing",
            |app| app.native_save_is_writing(),
        );
        // The save waits for `lock`, released below: a frame that waited for
        // it would never end.
        for _ in 0..8 {
            harness.step();
            let label = harness.state().file_activity_label().unwrap();
            assert!(
                harness
                    .query_all(egui_kittest::kittest::by().label(&label))
                    .next()
                    .is_some()
            );
            assert!(
                !harness.output().viewport_output[&ViewportId::ROOT]
                    .commands
                    .contains(&ViewportCommand::Close)
            );
        }
        assert!(!path.exists());
        if concurrent_edit {
            assert!(harness.state_mut().create_box());
        }
        lock.unlock().unwrap();
        step_until(
            &mut harness,
            Duration::from_secs(10),
            "save did not finish",
            |app| !app.native_save_in_progress(),
        );
        assert!(path.is_file());
        assert_eq!(harness.state().is_dirty(), concurrent_edit);
        assert_eq!(harness.state().close_guard.pending, concurrent_edit);
        assert_eq!(
            harness.output().viewport_output[&ViewportId::ROOT]
                .commands
                .contains(&ViewportCommand::Close),
            !concurrent_edit
        );
        close_event(&mut harness);
        assert_eq!(canceled(&harness), concurrent_edit);
    }
}

#[test]
fn saving_elsewhere_during_close_preparation_does_not_authorize_closing() {
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("requested.ketchup");
    let elsewhere = directory.path().join("elsewhere.ketchup");
    let mut harness = shell(ScriptedFileDialogs::new().queue_save(&requested), true);
    close_event(&mut harness);
    harness.run();
    let save = harness.state().catalog.text("file-save");
    harness.get_by_role_and_label(Role::Button, &save).click();
    harness.step();
    assert!(harness.state().native_save_in_progress());
    assert!(harness.state_mut().save_document_to(&elsewhere));
    assert!(!harness.state().is_dirty());
    step_until(
        &mut harness,
        Duration::from_secs(10),
        "save did not finish",
        |app| !app.native_save_in_progress(),
    );
    assert!(elsewhere.is_file());
    assert!(!requested.exists());
    assert!(harness.state().close_guard.pending);
    assert!(harness.state().close_guard.approved.is_none());
    assert!(
        !harness.output().viewport_output[&ViewportId::ROOT]
            .commands
            .contains(&ViewportCommand::Close)
    );
}

#[test]
fn save_on_close_writes_the_own_file_without_asking() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.ketchup");
    let script = ScriptedFileDialogs::new();
    let mut harness = shell(script.clone(), false);
    assert!(harness.state_mut().save_document_to(&path));
    let before = std::fs::read(&path).unwrap();
    assert!(harness.state_mut().create_box());
    close_event(&mut harness);
    click(&mut harness, "file-save");
    assert!(!harness.state().is_dirty());
    assert!(script.high_risk_prompts().is_empty());
    assert_ne!(std::fs::read(path).unwrap(), before);
}

#[test]
fn escape_cancels_close_and_repeated_native_close_keeps_prompting() {
    let mut harness = shell(ScriptedFileDialogs::new(), true);
    close_event(&mut harness);
    assert!(canceled(&harness));
    harness.run();
    close_event(&mut harness);
    assert!(canceled(&harness));
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().close_guard.pending);
    assert!(harness.state().close_guard.approved.is_none());
    assert!(harness.state().is_dirty());
}
