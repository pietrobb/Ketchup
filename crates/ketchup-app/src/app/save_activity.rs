//! Native saving: prepare and publish on workers, ask for consent on the UI thread.
use crate::*;
use ketchup_model::persistence::{
    self, ContainerData, FileIdentity, FilePersistenceError, PersistenceError, WorkRecoveryLock,
};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) type PreparedSave = Result<(Vec<u8>, bool), PersistenceError>;

pub(super) fn prepare_save(document: &DocumentStore, container: &ContainerData) -> PreparedSave {
    match persistence::save_document_store(document, container) {
        Ok(bytes) => Ok((bytes, false)),
        Err(PersistenceError::ResourceLimit | PersistenceError::TooLarge { .. }) => {
            persistence::save_document_store_current_snapshot(document, container)
                .map(|bytes| (bytes, true))
        }
        Err(error) => Err(error),
    }
}

struct Source {
    document: DocumentId,
    history: String,
    conversation: String,
    container: ContainerData,
    original_path: Option<PathBuf>,
}

impl Source {
    fn capture(app: &KetchupApp) -> Self {
        Self {
            document: app.document.current().document_id(),
            history: app.document.history_digest(),
            conversation: assistant_conversation_digest(&app.assistant.messages),
            container: app.file.container_data.clone(),
            original_path: app.file.path.clone(),
        }
    }

    fn current(&self, app: &KetchupApp) -> bool {
        self.document == app.document.current().document_id()
            && self.history == app.document.history_digest()
            && self.conversation == assistant_conversation_digest(&app.assistant.messages)
            && self.container == app.file.container_data
            && self.original_path == app.file.path
    }
}

enum Stage {
    Preparing(Receiver<PreparedSave>),
    Writing {
        receiver: Receiver<(Result<(), FilePersistenceError>, Option<WorkRecoveryLock>)>,
        identity: FileIdentity,
        truncate: bool,
    },
}

pub(super) struct SaveTask {
    path: PathBuf,
    source: Source,
    stage: Stage,
    close_requested: bool,
}

impl KetchupApp {
    pub(crate) fn start_background_save(&mut self, path: PathBuf, context: &egui::Context) {
        self.queue_save_preparation(path, context, prepare_save);
    }

    fn queue_save_preparation(
        &mut self,
        path: PathBuf,
        context: &egui::Context,
        prepare: impl FnOnce(&DocumentStore, &ContainerData) -> PreparedSave + Send + 'static,
    ) {
        self.store_assistant_conversation();
        self.store_assistant_memory();
        let source = Source::capture(self);
        let document = self.document.fork_for_planning();
        let container = source.container.clone();
        let repaint = context.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            if sender.send(prepare(&document, &container)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.saving = Some(SaveTask {
            path,
            source,
            stage: Stage::Preparing(receiver),
            close_requested: false,
        });
    }

    pub(crate) fn native_save_in_progress(&self) -> bool {
        self.file.activity.saving.is_some()
    }

    pub(crate) fn native_save_is_writing(&self) -> bool {
        self.file
            .activity
            .saving
            .as_ref()
            .is_some_and(|task| matches!(task.stage, Stage::Writing { .. }))
    }

    pub(super) fn cancel_save_preparation(&mut self) {
        if !self.native_save_is_writing() {
            self.file.activity.saving = None;
        }
    }

    pub(super) fn defer_close_until_saved(&mut self, context: &egui::Context) {
        if self.native_save_is_writing() {
            self.file
                .activity
                .saving
                .as_mut()
                .expect("writing task")
                .close_requested = true;
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    pub(super) fn start_prepared_write(
        &mut self,
        path: &Path,
        bytes: Vec<u8>,
        truncate: bool,
        expected: Option<FileIdentity>,
        target_lock: Option<WorkRecoveryLock>,
        context: &egui::Context,
    ) {
        let source = Source::capture(self);
        let identity = FileIdentity::from_bytes(&bytes);
        let worker_path = path.to_owned();
        let repaint = context.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result =
                persistence::save_prepared_document_if_unchanged(&worker_path, &bytes, expected);
            if sender.send((result, target_lock)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.saving = Some(SaveTask {
            path: path.to_owned(),
            source,
            stage: Stage::Writing {
                receiver,
                identity,
                truncate,
            },
            close_requested: false,
        });
    }

    pub(super) fn poll_native_save(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.saving.take() else {
            return;
        };
        match &task.stage {
            Stage::Preparing(receiver) => {
                let prepared = match receiver.try_recv() {
                    Ok(prepared) => prepared,
                    Err(TryRecvError::Empty) => {
                        self.file.activity.saving = Some(task);
                        return;
                    }
                    Err(error @ TryRecvError::Disconnected) => {
                        self.report_file_persistence_error(
                            "error-save-document",
                            &task.path,
                            &FilePersistenceError::Io(std::io::Error::other(error)),
                        );
                        context.request_repaint();
                        return;
                    }
                };
                if task.source.current(self) {
                    let _ = self.save_document_prepared_while(
                        &task.path,
                        || true,
                        Some(prepared),
                        Some(context),
                    );
                }
            }
            Stage::Writing {
                receiver,
                identity,
                truncate,
            } => {
                let (result, target_lock) = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => {
                        self.file.activity.saving = Some(task);
                        return;
                    }
                    Err(error @ TryRecvError::Disconnected) => (
                        Err(FilePersistenceError::Io(std::io::Error::other(error))),
                        None,
                    ),
                };
                let written = result.is_ok();
                self.finish_close_save(written && task.source.current(self));
                if !task.source.current(self)
                    && task.source.document == self.document.current().document_id()
                    && let Err(error) = &result
                {
                    self.report_file_persistence_error("error-save-document", &task.path, error);
                }
                if task.source.current(self) {
                    let _ = self.finish_document_save(&task.path, *truncate, *identity, result);
                } else if task.source.document == self.document.current().document_id()
                    && task.source.original_path == self.file.path
                    && task.source.original_path.as_deref() == Some(task.path.as_path())
                {
                    // An edit made through the live API during the write is not saved.
                    if result.is_ok()
                        || result
                            .as_ref()
                            .err()
                            .is_some_and(|error| error.published_identity(&task.path).is_some())
                    {
                        self.file.path = Some(task.path.clone());
                        self.file.identity = Some(*identity);
                        self.file.saved_digest = task.source.history;
                        self.assistant.saved_conversation_digest = task.source.conversation;
                    }
                }
                if self.is_dirty()
                    && task.source.document == self.document.current().document_id()
                    && self.file.path.as_deref().or_else(|| {
                        self.file
                            .recovery_open
                            .as_ref()
                            .map(|recovery| recovery.requested_path.as_path())
                    }) == Some(task.path.as_path())
                    && self.file.work_recovery_lock.is_none()
                {
                    self.file.work_recovery_lock = target_lock;
                }
                if task.close_requested && written {
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        context.request_repaint();
    }
}

#[cfg(test)]
mod tests;
