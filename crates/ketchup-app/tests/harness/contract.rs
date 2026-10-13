//! The behaviour every manual tool shares, checked once for any tool.
//!
//! A tool describes its gesture as three steps ([`ToolGesture`]); the checks
//! here cut in between them and assert on document state only — revision,
//! canonical digest and Undo/Redo depth before and after — never on painted
//! text. [`assert_tool_contract`] runs every check, each in a fresh shell, so
//! one broken property cannot hide another.

use super::Shell;
use eframe::egui::Key;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;

/// The document state a tool may only change by committing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentState {
    pub revision: u64,
    pub digest: String,
    pub undo_steps: usize,
    pub redo_steps: usize,
}

impl DocumentState {
    pub fn of(shell: &Shell) -> Self {
        let app = shell.app();
        Self {
            revision: app.document_revision(),
            digest: app.canonical_digest(),
            undo_steps: app.undo_step_count(),
            redo_steps: app.redo_step_count(),
        }
    }
}

/// One manual tool gesture, split where the contract needs to cut in.
pub struct ToolGesture<'a> {
    /// Names the gesture in failure messages.
    pub name: &'a str,
    /// Select the input and activate the tool.
    pub arm: &'a dyn Fn(&mut Shell),
    /// Start the gesture and leave it showing a preview, uncommitted.
    pub preview: &'a dyn Fn(&mut Shell),
    /// Complete the gesture from its preview.
    pub commit: &'a dyn Fn(&mut Shell),
}

/// Fails unless the document is exactly as it was in `before`.
pub fn assert_unchanged(shell: &Shell, before: &DocumentState, gesture: &str, after_what: &str) {
    assert_eq!(
        &DocumentState::of(shell),
        before,
        "{gesture}: {after_what} must leave the document unchanged"
    );
}

/// A preview changes nothing, and Escape drops it without a trace.
pub fn assert_preview_then_escape_change_nothing(shell: &mut Shell, gesture: &ToolGesture<'_>) {
    (gesture.arm)(shell);
    let before = DocumentState::of(shell);
    (gesture.preview)(shell);
    assert_unchanged(shell, &before, gesture.name, "the preview");
    shell.press_key(Key::Escape);
    assert_unchanged(shell, &before, gesture.name, "Escape during the preview");
}

/// Switching to `other` mid-gesture cancels it without a trace.
pub fn assert_tool_switch_changes_nothing(
    shell: &mut Shell,
    gesture: &ToolGesture<'_>,
    other: AppCommand,
) {
    (gesture.arm)(shell);
    let before = DocumentState::of(shell);
    (gesture.preview)(shell);
    shell.click_command(other);
    assert_unchanged(
        shell,
        &before,
        gesture.name,
        &format!("switching to {other:?} during the preview"),
    );
}

/// The commit is one revision and one Undo step; Undo and Redo return to the
/// exact digests before and after it. Returns the committed state.
pub fn assert_commit_is_one_undo_step(
    shell: &mut Shell,
    gesture: &ToolGesture<'_>,
) -> DocumentState {
    (gesture.arm)(shell);
    let before = DocumentState::of(shell);
    (gesture.preview)(shell);
    (gesture.commit)(shell);
    let after = DocumentState::of(shell);
    let name = gesture.name;
    assert_ne!(
        after.digest, before.digest,
        "{name}: the commit changed nothing"
    );
    assert_eq!(
        after.revision,
        before.revision + 1,
        "{name}: one commit, one revision"
    );
    assert_eq!(
        after.undo_steps,
        before.undo_steps + 1,
        "{name}: one commit, one Undo step"
    );
    assert_eq!(after.redo_steps, 0, "{name}: a commit clears Redo");

    shell.press_key(Key::Escape);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let undone = DocumentState::of(shell);
    assert_eq!(
        undone.digest, before.digest,
        "{name}: Undo must restore the exact document"
    );
    assert_eq!(
        undone.undo_steps, before.undo_steps,
        "{name}: Undo depth after Undo"
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let redone = DocumentState::of(shell);
    assert_eq!(
        redone.digest, after.digest,
        "{name}: Redo must restore the exact commit"
    );
    assert_eq!(
        redone.undo_steps, after.undo_steps,
        "{name}: Undo depth after Redo"
    );
    redone
}

/// A document change between preview and commit makes the commit a no-op:
/// the tool never applies a gesture measured against an older document.
pub fn assert_stale_commit_changes_nothing(
    shell: &mut Shell,
    gesture: &ToolGesture<'_>,
    change: &dyn Fn(&mut Shell),
) {
    (gesture.arm)(shell);
    (gesture.preview)(shell);
    let before_change = DocumentState::of(shell);
    change(shell);
    let changed = DocumentState::of(shell);
    assert_ne!(
        changed.digest, before_change.digest,
        "{}: the stale-making change must change the document",
        gesture.name
    );
    (gesture.commit)(shell);
    assert_unchanged(
        shell,
        &changed,
        gesture.name,
        "a commit measured against an older document",
    );
}

/// Dialogs that answer one Save As and one Open with `path`.
pub fn save_open_dialogs(path: &std::path::Path) -> ScriptedFileDialogs {
    ScriptedFileDialogs::new()
        .always_discard()
        .queue_save(path)
        .queue_open(path)
}

/// Save As, New, Open returns the exact digest. The shell must have been built
/// with [`save_open_dialogs`].
pub fn assert_survives_save_open(shell: &mut Shell, gesture: &str) {
    let saved = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    assert_ne!(
        shell.app().canonical_digest(),
        saved,
        "{gesture}: New must clear the document, or reopening proves nothing"
    );
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(
        shell.app().canonical_digest(),
        saved,
        "{gesture}: the committed result must survive Save and Open"
    );
}

/// Runs every check on `gesture`, each in a fresh shell from `new_shell`.
///
/// `switch_to` is the tool the switch check moves to; `stale_change` changes
/// the document the way another client would while the gesture is open.
pub fn assert_tool_contract(
    new_shell: &dyn Fn(ScriptedFileDialogs) -> Shell,
    gesture: &ToolGesture<'_>,
    switch_to: AppCommand,
    stale_change: &dyn Fn(&mut Shell),
) {
    assert_preview_then_escape_change_nothing(&mut new_shell(ScriptedFileDialogs::new()), gesture);
    assert_tool_switch_changes_nothing(
        &mut new_shell(ScriptedFileDialogs::new()),
        gesture,
        switch_to,
    );
    assert_stale_commit_changes_nothing(
        &mut new_shell(ScriptedFileDialogs::new()),
        gesture,
        stale_change,
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tool-contract.ketchup");
    let mut shell = new_shell(save_open_dialogs(&path));
    assert_commit_is_one_undo_step(&mut shell, gesture);
    assert_survives_save_open(&mut shell, gesture.name);
}
