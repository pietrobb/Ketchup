//! The seam between the file workflow and the operating system dialogs.
//!
//! The shell asks for a path and for permission to discard unsaved work
//! through [`FileDialogs`]. The desktop build answers with real `rfd` windows;
//! acceptance tests answer from a script, so the complete New/Open/Save/Save As
//! workflow can be replayed offscreen without a human at the keyboard.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The localized labels a dialog needs, resolved by the shell before asking.
pub struct SaveRequest<'a> {
    /// Human readable name of the `.ketchup` file filter.
    pub filter_label: &'a str,
    /// The file name the dialog should propose.
    pub suggested_name: &'a str,
}

/// The localized text of the unsaved-changes confirmation.
pub struct DiscardRequest<'a> {
    /// Dialog title.
    pub title: &'a str,
    /// Dialog body asking whether the unsaved work may be discarded.
    pub description: &'a str,
}

/// How the shell obtains file paths and destructive-action confirmation.
pub trait FileDialogs {
    /// Ask for an existing document to open. `None` cancels the command.
    fn pick_open_path(&mut self, filter_label: &str) -> Option<PathBuf>;

    /// Ask where to write the document. `None` cancels the command.
    fn pick_save_path(&mut self, request: SaveRequest<'_>) -> Option<PathBuf>;

    /// Ask whether unsaved changes in the active document may be discarded.
    fn confirm_discard(&mut self, request: DiscardRequest<'_>) -> bool;
}

/// The desktop implementation backed by native operating system dialogs.
#[derive(Debug, Default)]
pub struct NativeFileDialogs;

impl FileDialogs for NativeFileDialogs {
    fn pick_open_path(&mut self, filter_label: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .add_filter(filter_label, &["ketchup"])
            .pick_file()
    }

    fn pick_save_path(&mut self, request: SaveRequest<'_>) -> Option<PathBuf> {
        let path = rfd::FileDialog::new()
            .add_filter(request.filter_label, &["ketchup"])
            .set_file_name(request.suggested_name)
            .save_file()?;
        if path.extension().is_none() {
            Some(path.with_extension("ketchup"))
        } else {
            Some(path)
        }
    }

    fn confirm_discard(&mut self, request: DiscardRequest<'_>) -> bool {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(request.title)
            .set_description(request.description)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
    }
}

#[derive(Debug, Default)]
struct ScriptState {
    open_paths: VecDeque<Option<PathBuf>>,
    save_paths: VecDeque<Option<PathBuf>>,
    discard: bool,
    suggested_names: Vec<String>,
    discard_prompts: usize,
}

/// A scripted stand-in for the native dialogs, used by acceptance tests.
///
/// Answers are queued in advance; an exhausted queue answers "cancelled",
/// which is exactly how a user dismissing the dialog behaves.
#[derive(Clone, Debug, Default)]
pub struct ScriptedFileDialogs(Arc<Mutex<ScriptState>>);

impl ScriptedFileDialogs {
    /// A script that cancels every dialog and refuses to discard changes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue the next answer of the Open dialog.
    #[must_use]
    pub fn queue_open(self, path: impl Into<PathBuf>) -> Self {
        self.state().open_paths.push_back(Some(path.into()));
        self
    }

    /// Queue the next answer of the Save dialog.
    #[must_use]
    pub fn queue_save(self, path: impl Into<PathBuf>) -> Self {
        self.state().save_paths.push_back(Some(path.into()));
        self
    }

    /// Queue a cancelled Save dialog.
    #[must_use]
    pub fn queue_cancelled_save(self) -> Self {
        self.state().save_paths.push_back(None);
        self
    }

    /// Answer every unsaved-changes prompt with "discard".
    #[must_use]
    pub fn always_discard(self) -> Self {
        self.state().discard = true;
        self
    }

    /// The file names the shell proposed to the Save dialog, in order.
    #[must_use]
    pub fn suggested_names(&self) -> Vec<String> {
        self.state().suggested_names.clone()
    }

    /// How many times the shell asked before discarding unsaved work.
    #[must_use]
    pub fn discard_prompts(&self) -> usize {
        self.state().discard_prompts
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ScriptState> {
        self.0.lock().expect("the scripted dialog state is intact")
    }
}

impl FileDialogs for ScriptedFileDialogs {
    fn pick_open_path(&mut self, _filter_label: &str) -> Option<PathBuf> {
        self.state().open_paths.pop_front().flatten()
    }

    fn pick_save_path(&mut self, request: SaveRequest<'_>) -> Option<PathBuf> {
        let mut state = self.state();
        state
            .suggested_names
            .push(request.suggested_name.to_owned());
        state.save_paths.pop_front().flatten()
    }

    fn confirm_discard(&mut self, _request: DiscardRequest<'_>) -> bool {
        let mut state = self.state();
        state.discard_prompts += 1;
        state.discard
    }
}
