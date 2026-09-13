use crate::{AssistantChatMessage, AssistantMessageRole, KetchupApp, dialogs::ScriptedFileDialogs};
use eframe::egui::{self, ViewportCommand, ViewportEvent, ViewportId, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable as _};

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
    assert_eq!(
        harness.output().viewport_output[&ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, ViewportCommand::Close)),
        harness.state().close_guard.approved.is_some()
    );
    harness.run();
}

fn chat(app: &mut KetchupApp) {
    app.assistant_messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Keep this unsaved conversation.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
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
fn conversation_only_changes_also_block_close() {
    let mut harness = shell(ScriptedFileDialogs::new(), false);
    let before = harness.state().document.history_digest();
    chat(harness.state_mut());
    assert!(harness.state().is_dirty());
    close_event(&mut harness);
    assert!(canceled(&harness));
    click(&mut harness, "action-cancel");
    assert_eq!(harness.state().assistant_messages.len(), 1);
    assert_eq!(harness.state().document.history_digest(), before);
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
fn discard_does_not_authorize_later_model_or_conversation_changes() {
    for change_conversation in [false, true] {
        let mut harness = shell(ScriptedFileDialogs::new(), true);
        close_event(&mut harness);
        click(&mut harness, "dialog-close-discard");
        if change_conversation {
            chat(harness.state_mut());
        } else {
            assert!(harness.state_mut().create_box());
        }
        close_event(&mut harness);
        assert!(canceled(&harness));
        assert!(harness.state().close_guard.pending);
        assert!(harness.state().close_guard.approved.is_none());
    }
}

#[test]
fn save_on_close_preserves_model_conversation_and_undo_on_disk() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("close-save.ketchup");
    let mut harness = shell(ScriptedFileDialogs::new().queue_save(&path), true);
    chat(harness.state_mut());
    let history = harness.state().document.history_digest();
    let conversation = harness.state().assistant_messages.clone();
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
    assert_eq!(reopened.assistant_messages, conversation);
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
fn refused_overwrite_on_close_does_not_change_disk_or_close() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.ketchup");
    let mut harness = shell(ScriptedFileDialogs::new(), false);
    assert!(harness.state_mut().save_document_to(&path));
    let before = std::fs::read(&path).unwrap();
    assert!(harness.state_mut().create_box());
    close_event(&mut harness);
    click(&mut harness, "file-save");
    assert!(harness.state().is_dirty());
    assert!(harness.state().close_guard.pending);
    assert!(harness.state().close_guard.approved.is_none());
    assert_eq!(std::fs::read(path).unwrap(), before);
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
