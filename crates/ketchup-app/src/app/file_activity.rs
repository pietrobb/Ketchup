//! File activity keeps the previous viewport visible while loading on a worker.
//! Other commands present their indication before entering the file operation.

use crate::*;

#[derive(Default)]
pub(crate) struct FileActivity {
    pub(crate) context: Option<egui::Context>,
    pending: Option<Pending>,
    opening: Option<OpenTask>,
    pub(super) saving: Option<super::save_activity::SaveTask>,
    pub(super) viewer_export: Option<super::viewer_export_activity::ViewerExportTask>,
    pub(super) exchange_export: Option<super::exchange_export::ExchangeExportTask>,
    pub(super) dwg_reading: Option<super::dwg_import::DwgReadTask>,
}

type OpenResult = Result<
    ketchup_model::persistence::LoadedFile,
    ketchup_model::persistence::FilePersistenceError,
>;

struct OpenTask {
    path: PathBuf,
    document: DocumentId,
    revision: u64,
    receiver: std::sync::mpsc::Receiver<(
        OpenResult,
        Option<ketchup_model::persistence::WorkRecoveryLock>,
    )>,
}

struct Pending {
    command: AppCommand,
    path: Option<PathBuf>,
    frame: u64,
    document: DocumentId,
    revision: u64,
}

impl KetchupApp {
    /// Queue a startup document; the native window paints before disk I/O begins.
    pub fn queue_open_document_path(&mut self, path: &Path) {
        let snapshot = self.document.current();
        self.file.activity.pending = Some(Pending {
            command: AppCommand::Open,
            path: Some(path.to_owned()),
            frame: self
                .file
                .activity
                .context
                .as_ref()
                .map_or(0, egui::Context::cumulative_frame_nr),
            document: snapshot.document_id(),
            revision: snapshot.revision_id(),
        });
    }

    pub(crate) fn start_document_read(
        &mut self,
        path: PathBuf,
        recovery_lock: Option<ketchup_model::persistence::WorkRecoveryLock>,
        context: &egui::Context,
    ) {
        self.queue_document_read(path, recovery_lock, context, |path| {
            ketchup_model::persistence::load_file_with_source(path)
        });
    }

    fn queue_document_read(
        &mut self,
        path: PathBuf,
        recovery_lock: Option<ketchup_model::persistence::WorkRecoveryLock>,
        context: &egui::Context,
        load: impl FnOnce(&Path) -> OpenResult + Send + 'static,
    ) {
        let snapshot = self.document.current();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_path = path.clone();
        let repaint = context.clone();
        std::thread::spawn(move || {
            let result = load(&worker_path);
            if sender.send((result, recovery_lock)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.opening = Some(OpenTask {
            path,
            document: snapshot.document_id(),
            revision: snapshot.revision_id(),
            receiver,
        });
    }

    fn poll_document_read(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.opening.as_ref() else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(error @ std::sync::mpsc::TryRecvError::Disconnected) => (
                Err(ketchup_model::persistence::FilePersistenceError::Io(
                    std::io::Error::other(error),
                )),
                None,
            ),
        };
        let task = self.file.activity.opening.take().expect("polled task");
        let snapshot = self.document.current();
        if task.document == snapshot.document_id() && task.revision == snapshot.revision_id() {
            self.install_loaded_document(&task.path, result.1, result.0);
        }
        context.request_repaint();
    }

    pub(crate) fn defer_file_activity(&mut self, command: AppCommand) -> bool {
        if self.file.activity.pending.is_some()
            || self.file.activity.opening.is_some()
            || self.file.activity.saving.is_some()
            || self.file.activity.viewer_export.is_some()
            || self.file.activity.exchange_export.is_some()
            || self.file.activity.dwg_reading.is_some()
        {
            return true;
        }
        let Some(context) = &self.file.activity.context else {
            return false;
        };
        if !matches!(
            command,
            AppCommand::Open
                | AppCommand::Save
                | AppCommand::SaveAs
                | AppCommand::ExportDrawingDxf
                | AppCommand::ExportExactStep
                | AppCommand::ExportExactIges
                | AppCommand::ExportMeshStl
                | AppCommand::ExportPrintThreeMf
                | AppCommand::ExportBlenderGlb
                | AppCommand::ExportViewer
                | AppCommand::ExportGeneralFabrication
                | AppCommand::ExportWeldmentCutList
                | AppCommand::ExportSheetMetalManufacturing
                | AppCommand::ExportHomagMpr
                | AppCommand::ExportHundeggerBtlx
        ) {
            return false;
        }
        let snapshot = self.document.current();
        self.file.activity.pending = Some(Pending {
            command,
            path: None,
            frame: context.cumulative_frame_nr(),
            document: snapshot.document_id(),
            revision: snapshot.revision_id(),
        });
        context.request_repaint();
        true
    }

    pub(crate) fn file_activity_label(&self) -> Option<String> {
        let command = if self.file.activity.opening.is_some() {
            AppCommand::Open
        } else if self.file.activity.dwg_reading.is_some() {
            AppCommand::ImportDrawingDxf
        } else if self.file.activity.viewer_export.is_some() {
            AppCommand::ExportViewer
        } else if let Some(task) = &self.file.activity.exchange_export {
            task.format().command()
        } else if self.file.activity.saving.is_some() {
            AppCommand::Save
        } else {
            self.file.activity.pending.as_ref()?.command
        };
        Some(format!(
            "{}…",
            self.command_label(command).trim_end_matches(['.', '…'])
        ))
    }

    pub(crate) fn file_write_in_progress(&self) -> bool {
        self.native_save_is_writing() || self.viewer_export_is_writing()
    }

    pub(crate) fn finish_file_activity_frame(&mut self, context: &egui::Context) {
        self.file.activity.context = None;
        if context.input(|input| input.viewport().close_requested()) {
            self.file.activity.pending = None;
            self.file.activity.opening = None;
            self.cancel_dwg_read();
            self.defer_close_until_saved(context);
            self.defer_close_until_exported(context);
            self.cancel_save_preparation();
            self.cancel_viewer_export_preparation();
            self.cancel_exchange_export();
        }
        let Some(label) = self.file_activity_label() else {
            return;
        };
        let response = egui::Modal::new(egui::Id::new("file-activity")).show(context, |ui| {
            // The spinner keeps frames coming, so the finished worker is
            // picked up without waiting for unrelated input.
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(label);
            });
            ui.add_enabled(
                !self.file_write_in_progress(),
                egui::Button::new(self.catalog.text("action-cancel")),
            )
            .clicked()
        });
        if (response.inner || response.should_close()) && !self.file_write_in_progress() {
            self.file.activity.pending = None;
            self.file.activity.opening = None;
            self.cancel_dwg_read();
            self.cancel_save_preparation();
            self.cancel_viewer_export_preparation();
            self.cancel_exchange_export();
            context.request_repaint();
            return;
        }
        if self.file.activity.exchange_export.is_some() {
            self.poll_exchange_export(context);
            return;
        }
        if self.file.activity.saving.is_some() {
            self.poll_native_save(context);
            return;
        }
        if self.file.activity.viewer_export.is_some() {
            self.poll_viewer_export(context);
            return;
        }
        if self.file.activity.opening.is_some() {
            self.poll_document_read(context);
            return;
        }
        if self.file.activity.dwg_reading.is_some() {
            self.poll_dwg_read(context);
            return;
        }
        let pending = self
            .file
            .activity
            .pending
            .as_ref()
            .expect("visible activity");
        // egui may run several layout passes in one frame. Only a later frame
        // guarantees that the native backend has had a chance to paint the label.
        if pending.frame >= context.cumulative_frame_nr() {
            context.request_repaint();
            return;
        }
        let pending = self.file.activity.pending.take().expect("visible activity");
        let snapshot = self.document.current();
        if pending.document == snapshot.document_id() && pending.revision == snapshot.revision_id()
        {
            if pending.command == AppCommand::Open {
                if self.confirm_discard_if_dirty()
                    && let Some(path) = pending.path.or_else(|| self.choose_open_path())
                {
                    self.open_document_in_context(&path, true, Some(context));
                }
            } else if matches!(pending.command, AppCommand::Save | AppCommand::SaveAs) {
                let path = if pending.command == AppCommand::Save {
                    self.file.path.clone()
                } else {
                    None
                };
                if let Some(path) = path.or_else(|| self.choose_save_path()) {
                    self.start_background_save(path, context);
                }
            } else if pending.command == AppCommand::ExportViewer {
                if let Some(path) = self.choose_export_path("ketchup-view") {
                    self.start_viewer_export(path, context);
                }
            } else if let Some(format) = match pending.command {
                AppCommand::ExportExactStep => Some(super::exchange_export::ExchangeFormat::Step),
                AppCommand::ExportExactIges => Some(super::exchange_export::ExchangeFormat::Iges),
                _ => None,
            } {
                if let Some(path) = self.choose_export_path(format.key()) {
                    self.start_exchange_export(format, path, context);
                }
            } else {
                self.dispatch_command(pending.command);
            }
        }
        context.request_repaint();
    }
}

#[cfg(test)]
mod tests {
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

    fn wait_for_open(harness: &mut Harness<'_, KetchupApp>) {
        step_until(
            harness,
            Duration::from_secs(10),
            "open did not finish",
            |app| {
                app.file.activity.opening.is_none()
                    && app.file.activity.pending.is_none()
                    && app.file.activity.saving.is_none()
            },
        );
    }

    fn gated_read(
        harness: &mut Harness<'_, KetchupApp>,
        path: PathBuf,
    ) -> (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>) {
        let (release, gate) = std::sync::mpsc::channel();
        let (finished, completion) = std::sync::mpsc::channel();
        let context = harness.ctx.clone();
        harness
            .state_mut()
            .queue_document_read(path, None, &context, move |path| {
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
                let result = ketchup_model::persistence::load_file_with_source(path);
                finished.send(()).unwrap();
                result
            });
        (release, completion)
    }

    #[test]
    fn open_shortcut_loads_the_saved_model_without_blocking_the_command_frame() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("open.ketchup");
        let mut source = KetchupApp::new();
        assert!(source.create_box());
        assert!(source.save_document_to(&path));
        let expected = source.document.current().occurrences().count();
        let mut harness = shell(ScriptedFileDialogs::new().queue_open(&path));
        harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::O,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        harness.step();
        let label = harness.state().file_activity_label().unwrap();
        assert!(
            harness
                .query_all(egui_kittest::kittest::by().label(&label))
                .next()
                .is_some()
        );
        harness.step();
        assert!(harness.state().file.activity.opening.is_some());
        wait_for_open(&mut harness);
        assert_eq!(
            harness.state().document.current().occurrences().count(),
            expected
        );
        assert_eq!(harness.state().file.path.as_deref(), Some(path.as_path()));
        assert!(!harness.state().is_dirty());
    }

    #[test]
    fn startup_paints_before_reading_and_failed_open_keeps_the_current_model() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.ketchup");
        let mut app = KetchupApp::new();
        let before = app.document.history_digest();
        app.queue_open_document_path(&missing);
        let mut first_frame = true;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1600.0, 1000.0))
            .build_state(
                move |ctx, app: &mut KetchupApp| {
                    app.ui(ctx);
                    if first_frame {
                        assert!(app.file.activity.pending.is_some());
                        assert!(app.file.activity.opening.is_none());
                        assert!(app.file_activity_label().is_some());
                        first_frame = false;
                    }
                },
                app,
            );
        wait_for_open(&mut harness);
        assert_eq!(harness.state().document.history_digest(), before);
        assert!(harness.state().file.path.is_none());
        assert!(harness.state().digest.contains("missing.ketchup"));
    }

    #[test]
    fn startup_path_opens_the_existing_document_it_was_given() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("double-clicked.ketchup");
        let mut source = KetchupApp::new();
        assert!(source.create_box());
        assert!(source.save_document_to(&path));
        let expected = source.document.current().occurrences().count();
        let mut app = KetchupApp::new();
        app.queue_open_document_path(&path);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1600.0, 1000.0))
            .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app);
        wait_for_open(&mut harness);
        assert_eq!(harness.state().file.path.as_deref(), Some(path.as_path()));
        assert_eq!(
            harness.state().document.current().occurrences().count(),
            expected
        );
        assert!(expected > 0);
        assert!(!harness.state().is_dirty());
    }

    #[test]
    fn stalled_read_remains_visible_and_escape_or_close_preserves_unsaved_work() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("replacement.ketchup");
        assert!(KetchupApp::new().save_document_to(&path));
        for close in [false, true] {
            let mut harness = shell(ScriptedFileDialogs::new());
            assert!(harness.state_mut().create_box());
            let before = harness.state().document.history_digest();
            let (release, finished) = gated_read(&mut harness, path.clone());
            for _ in 0..8 {
                harness.step();
                let label = harness.state().file_activity_label().unwrap();
                assert!(
                    harness
                        .query_all(egui_kittest::kittest::by().label(&label))
                        .next()
                        .is_some()
                );
                assert_eq!(harness.state().document.history_digest(), before);
            }
            if close {
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
            assert!(harness.state().file.activity.opening.is_none());
            assert_eq!(harness.state().close_guard.pending, close);
            assert!(harness.state().is_dirty());
            release.send(()).unwrap();
            finished.recv_timeout(Duration::from_secs(10)).unwrap();
            harness.step();
            assert_eq!(harness.state().document.history_digest(), before);
        }
    }

    #[test]
    fn background_read_never_replaces_a_newer_edit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.ketchup");
        assert!(KetchupApp::new().save_document_to(&path));
        let mut harness = shell(ScriptedFileDialogs::new());
        let (release, finished) = gated_read(&mut harness, path);
        assert!(harness.state_mut().create_box());
        let before = harness.state().document.history_digest();
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(10)).unwrap();
        wait_for_open(&mut harness);
        assert_eq!(harness.state().document.history_digest(), before);
        assert!(harness.state().is_dirty());
        assert!(harness.state().file.path.is_none());
    }

    fn start_save(harness: &mut Harness<'_, KetchupApp>) {
        harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        harness.step();
        let label = harness.state().file_activity_label().expect("pending save");
        assert!(
            harness
                .query_all(egui_kittest::kittest::by().label(&label))
                .next()
                .is_some(),
            "save indication was drawn"
        );
    }

    #[test]
    fn save_is_indicated_before_writing_and_reopens_with_the_same_model() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("indicated.ketchup");
        let mut harness = shell(ScriptedFileDialogs::new().queue_save(&path));
        let before = harness.state().document.current().canonical_digest();
        start_save(&mut harness);
        assert!(
            !path.exists(),
            "first frame must be presented before saving"
        );
        wait_for_open(&mut harness);
        assert!(harness.state().file_activity_label().is_none());
        let mut reopened = KetchupApp::new();
        assert!(reopened.open_document_path(&path));
        assert_eq!(reopened.document.current().canonical_digest(), before);
    }

    #[test]
    fn cancel_during_the_indication_does_not_write_or_discard_work() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cancelled.ketchup");
        let mut harness = shell(ScriptedFileDialogs::new().queue_save(&path));
        assert!(harness.state_mut().create_box());
        let before = harness.state().document.history_digest();
        start_save(&mut harness);
        harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        assert!(!path.exists());
        assert!(harness.state().is_dirty());
        assert_eq!(harness.state().document.history_digest(), before);
        assert!(harness.state().file_activity_label().is_none());
    }

    #[test]
    fn native_close_during_the_indication_keeps_the_unsaved_work_guard() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("closed.ketchup");
        let mut harness = shell(ScriptedFileDialogs::new().queue_save(&path));
        assert!(harness.state_mut().create_box());
        start_save(&mut harness);
        harness
            .input_mut()
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        harness.step();
        assert!(!path.exists());
        assert!(harness.state().close_guard.pending);
        assert!(harness.state().is_dirty());
        assert!(harness.state().file_activity_label().is_none());
        assert!(
            harness.output().viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::CancelClose)
        );
    }
}
