//! The seam between the file workflow and the operating system dialogs.
//!
//! The shell asks for a path and for permission to discard unsaved work
//! through [`FileDialogs`]. The desktop build answers with real `rfd` windows;
//! acceptance tests answer from a script, so the complete New/Open/Save/Save As
//! workflow can be replayed offscreen without a human at the keyboard.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};

/// The localized labels a dialog needs, resolved by the shell before asking.
pub struct SaveRequest<'a> {
    /// Human readable name of the `.ketchup` file filter.
    pub filter_label: &'a str,
    /// The file name the dialog should propose.
    pub suggested_name: &'a str,
}

pub struct ExportRequest<'a> {
    pub filter_label: &'a str,
    pub extension: &'a str,
    pub suggested_name: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportRequestRecord {
    pub filter_label: String,
    pub extension: String,
    pub suggested_name: String,
}

/// The localized text of the unsaved-changes confirmation.
pub struct DiscardRequest<'a> {
    /// Dialog title.
    pub title: &'a str,
    /// Dialog body asking whether the unsaved work may be discarded.
    pub description: &'a str,
}

pub struct HighRiskConfirmationRequest<'a> {
    pub title: &'a str,
    pub description: &'a str,
}

/// How the shell obtains file paths and destructive-action confirmation.
pub trait FileDialogs {
    /// Ask for an existing document to open. `None` cancels the command.
    fn pick_open_path(&mut self, filter_label: &str) -> Option<PathBuf>;

    /// Ask where to write the document. `None` cancels the command.
    fn pick_save_path(&mut self, request: SaveRequest<'_>) -> Option<PathBuf>;

    fn pick_export_path(&mut self, request: ExportRequest<'_>) -> Option<PathBuf>;

    /// Ask whether unsaved changes in the active document may be discarded.
    fn confirm_discard(&mut self, request: DiscardRequest<'_>) -> bool;

    /// Return the authenticated local human who approved the exact high-risk evidence.
    fn confirm_high_risk(&mut self, request: HighRiskConfirmationRequest<'_>) -> Option<u64>;
}

/// The main window a native dialog is owned by.
///
/// Windows only treats a common item dialog as modal when it is given an owner:
/// the owner is disabled while the dialog is up, and the dialog can never fall
/// behind it. Without an owner the Save As window is a free floating top level
/// window that can be lost behind the model view.
#[derive(Clone, Copy, Debug)]
pub struct DialogParentWindow {
    window: RawWindowHandle,
    display: RawDisplayHandle,
}

impl DialogParentWindow {
    /// The handles of the window `eframe` just created, when the platform has them.
    #[must_use]
    pub fn from_creation_context(context: &eframe::CreationContext<'_>) -> Option<Self> {
        Some(Self {
            window: context.window_handle().ok()?.as_raw(),
            display: context.display_handle().ok()?.as_raw(),
        })
    }
}

impl HasWindowHandle for DialogParentWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the handle is the main window `eframe` created before the app
        // existed and destroys after it is dropped, so it is valid for every
        // borrow taken through `&self`.
        #[allow(unsafe_code)]
        unsafe {
            Ok(WindowHandle::borrow_raw(self.window))
        }
    }
}

impl HasDisplayHandle for DialogParentWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: the display outlives the main window it was read from, and so
        // outlives every borrow taken through `&self`.
        #[allow(unsafe_code)]
        unsafe {
            Ok(DisplayHandle::borrow_raw(self.display))
        }
    }
}

/// The desktop implementation backed by native operating system dialogs.
#[derive(Debug, Default)]
pub struct NativeFileDialogs {
    parent: Option<DialogParentWindow>,
}

impl NativeFileDialogs {
    /// Native dialogs owned by, and modal to, the given main window.
    #[must_use]
    pub const fn with_parent(parent: Option<DialogParentWindow>) -> Self {
        Self { parent }
    }

    fn file_dialog(&self) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match self.parent.as_ref() {
            Some(parent) => dialog.set_parent(parent),
            None => dialog,
        }
    }

    fn message_dialog(&self) -> rfd::MessageDialog {
        let dialog = rfd::MessageDialog::new();
        match self.parent.as_ref() {
            Some(parent) => dialog.set_parent(parent),
            None => dialog,
        }
    }
}

impl FileDialogs for NativeFileDialogs {
    fn pick_open_path(&mut self, filter_label: &str) -> Option<PathBuf> {
        self.file_dialog()
            .add_filter(filter_label, &["ketchup"])
            .pick_file()
    }

    fn pick_save_path(&mut self, request: SaveRequest<'_>) -> Option<PathBuf> {
        let path = self
            .file_dialog()
            .add_filter(request.filter_label, &["ketchup"])
            .set_file_name(request.suggested_name)
            .save_file()?;
        if path.extension().is_none() {
            Some(path.with_extension("ketchup"))
        } else {
            Some(path)
        }
    }

    fn pick_export_path(&mut self, request: ExportRequest<'_>) -> Option<PathBuf> {
        let path = self
            .file_dialog()
            .add_filter(request.filter_label, &[request.extension])
            .set_file_name(request.suggested_name)
            .save_file()?;
        if path.extension().is_none() {
            Some(path.with_extension(request.extension))
        } else {
            Some(path)
        }
    }

    fn confirm_discard(&mut self, request: DiscardRequest<'_>) -> bool {
        self.message_dialog()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(request.title)
            .set_description(request.description)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
    }

    fn confirm_high_risk(&mut self, request: HighRiskConfirmationRequest<'_>) -> Option<u64> {
        (self
            .message_dialog()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(request.title)
            .set_description(request.description)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes)
            .then_some(1)
    }
}

#[derive(Debug, Default)]
struct ScriptState {
    open_paths: VecDeque<Option<PathBuf>>,
    save_paths: VecDeque<Option<PathBuf>>,
    export_paths: VecDeque<Option<PathBuf>>,
    discard: bool,
    high_risk_approvals: VecDeque<Option<u64>>,
    default_high_risk_approver: Option<u64>,
    suggested_names: Vec<String>,
    export_requests: Vec<ExportRequestRecord>,
    discard_prompts: usize,
    high_risk_prompts: Vec<String>,
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

    #[must_use]
    pub fn queue_export(self, path: impl Into<PathBuf>) -> Self {
        self.state().export_paths.push_back(Some(path.into()));
        self
    }

    #[must_use]
    pub fn queue_cancelled_export(self) -> Self {
        self.state().export_paths.push_back(None);
        self
    }

    /// Answer every unsaved-changes prompt with "discard".
    #[must_use]
    pub fn always_discard(self) -> Self {
        self.state().discard = true;
        self
    }

    #[must_use]
    pub fn queue_refused_high_risk(self) -> Self {
        self.state().high_risk_approvals.push_back(None);
        self
    }

    #[must_use]
    pub fn queue_high_risk_approval(self, human_id: u64) -> Self {
        self.state().high_risk_approvals.push_back(Some(human_id));
        self
    }

    #[must_use]
    pub fn always_confirm_high_risk_as(self, human_id: u64) -> Self {
        self.state().default_high_risk_approver = Some(human_id);
        self
    }

    #[must_use]
    pub fn high_risk_prompts(&self) -> Vec<String> {
        self.state().high_risk_prompts.clone()
    }

    /// The file names the shell proposed to the Save dialog, in order.
    #[must_use]
    pub fn suggested_names(&self) -> Vec<String> {
        self.state().suggested_names.clone()
    }

    #[must_use]
    pub fn export_requests(&self) -> Vec<ExportRequestRecord> {
        self.state().export_requests.clone()
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

    fn pick_export_path(&mut self, request: ExportRequest<'_>) -> Option<PathBuf> {
        let mut state = self.state();
        state
            .suggested_names
            .push(request.suggested_name.to_owned());
        state.export_requests.push(ExportRequestRecord {
            filter_label: request.filter_label.to_owned(),
            extension: request.extension.to_owned(),
            suggested_name: request.suggested_name.to_owned(),
        });
        if state.export_paths.is_empty() {
            state.save_paths.pop_front().flatten()
        } else {
            state.export_paths.pop_front().flatten()
        }
    }

    fn confirm_discard(&mut self, _request: DiscardRequest<'_>) -> bool {
        let mut state = self.state();
        state.discard_prompts += 1;
        state.discard
    }

    fn confirm_high_risk(&mut self, request: HighRiskConfirmationRequest<'_>) -> Option<u64> {
        let mut state = self.state();
        state
            .high_risk_prompts
            .push(format!("{}\n{}", request.title, request.description));
        state
            .high_risk_approvals
            .pop_front()
            .unwrap_or(state.default_high_risk_approver)
    }
}
