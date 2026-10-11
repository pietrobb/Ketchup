//! Prepare Viewer packages and publish authorized artifact pairs without blocking UI.
use super::viewer_export::ViewerExportInput;
use super::viewer_settings::ViewerSettings;
use crate::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) struct PreparedExport {
    bytes: Vec<u8>,
    report: String,
    evidence: Vec<u8>,
}

impl PreparedExport {
    pub(super) fn prepare(path: &Path, input: ViewerExportInput) -> Result<Self, ExportError> {
        let bundle = input.build()?;
        let mut bytes = Vec::new();
        bundle.package.write(&mut bytes)?;
        let report_path = path.with_extension("ketchup-view.loss.txt");
        let evidence = super::export::export_bundle_evidence(
            b"ketchup.current-model-viewer-export.v1",
            path,
            &bytes,
            &report_path,
            bundle.loss_report.as_bytes(),
        );
        Ok(Self {
            bytes,
            report: bundle.loss_report,
            evidence,
        })
    }

    pub(super) fn authorize(
        &self,
        app: &mut KetchupApp,
        path: &Path,
        precondition: &ExportBundlePrecondition,
    ) -> Result<(), ExportError> {
        app.authorize_export_bundle(
            ExportConsent {
                class: HighRiskClass::LossyConversion,
                operation: "export-current-model-viewer-with-loss-report",
                title_key: "dialog-export-viewer-title",
                risk_key: "dialog-export-viewer-risk",
                overwrite_title_key: "dialog-export-viewer-overwrite-title",
                overwrite_operations: [
                    "overwrite-current-model-viewer-export",
                    "overwrite-current-model-viewer-loss-report",
                ],
            },
            path,
            &path.with_extension("ketchup-view.loss.txt"),
            precondition,
            &self.evidence,
        )
    }

    pub(super) fn write(
        self,
        path: &Path,
        precondition: &ExportBundlePrecondition,
    ) -> Result<(), ExportError> {
        Ok(write_export_bundle(
            path,
            &self.bytes,
            &path.with_extension("ketchup-view.loss.txt"),
            self.report.as_bytes(),
            precondition,
        )?)
    }
}

enum Stage {
    Preparing(Receiver<Result<PreparedExport, ExportError>>),
    Inspecting(Receiver<Result<(PreparedExport, ExportBundlePrecondition), ExportError>>),
    Writing(Receiver<Result<(), ExportError>>),
}

pub(super) struct ViewerExportTask {
    path: PathBuf,
    document: DocumentId,
    revision: u64,
    settings: ViewerSettings,
    stage: Stage,
    close_requested: bool,
}

impl KetchupApp {
    pub(super) fn start_viewer_export(&mut self, path: PathBuf, context: &egui::Context) {
        self.side_effect_receipts.clear();
        match self.viewer_file_input(&path) {
            Ok(input) => self.queue_viewer_export(path, input, context, PreparedExport::prepare),
            Err(error) => {
                self.report_export_outcome("viewer", &path, Err(error)).ok();
            }
        }
    }

    fn queue_viewer_export(
        &mut self,
        path: PathBuf,
        input: ViewerExportInput,
        context: &egui::Context,
        prepare: impl FnOnce(&Path, ViewerExportInput) -> Result<PreparedExport, ExportError>
        + Send
        + 'static,
    ) {
        let document = input.snapshot.document_id();
        let revision = input.snapshot.revision_id();
        let settings = input.settings.clone();
        let worker_path = path.clone();
        let repaint = context.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            if sender.send(prepare(&worker_path, input)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.viewer_export = Some(ViewerExportTask {
            path,
            document,
            revision,
            settings,
            stage: Stage::Preparing(receiver),
            close_requested: false,
        });
        context.request_repaint();
    }

    pub(crate) fn viewer_export_is_writing(&self) -> bool {
        self.file
            .activity
            .viewer_export
            .as_ref()
            .is_some_and(|task| !matches!(task.stage, Stage::Preparing(_)))
    }

    pub(super) fn cancel_viewer_export_preparation(&mut self) {
        if !self.viewer_export_is_writing() {
            self.file.activity.viewer_export = None;
        }
    }

    pub(super) fn defer_close_until_exported(&mut self, context: &egui::Context) {
        if self.viewer_export_is_writing() {
            self.file
                .activity
                .viewer_export
                .as_mut()
                .expect("writing export")
                .close_requested = true;
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    pub(super) fn poll_viewer_export(&mut self, context: &egui::Context) {
        let Some(mut task) = self.file.activity.viewer_export.take() else {
            return;
        };
        match &task.stage {
            Stage::Preparing(receiver) => {
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => {
                        self.file.activity.viewer_export = Some(task);
                        return;
                    }
                    Err(error @ TryRecvError::Disconnected) => Err(ExportError::failed(error)),
                };
                match result {
                    Ok(prepared) => {
                        let path = task.path.clone();
                        let repaint = context.clone();
                        let (sender, receiver) = mpsc::channel();
                        // Recovery can mutate an old transaction journal. Unlike pure
                        // package preparation, this phase must finish before another export.
                        std::thread::spawn(move || {
                            let result = ExportBundlePrecondition::capture(
                                &path,
                                &path.with_extension("ketchup-view.loss.txt"),
                            )
                            .map(|precondition| (prepared, precondition))
                            .map_err(ExportError::from);
                            if sender.send(result).is_ok() {
                                repaint.request_repaint();
                            }
                        });
                        task.stage = Stage::Inspecting(receiver);
                        self.file.activity.viewer_export = Some(task);
                    }
                    Err(error) => {
                        self.report_export_outcome("viewer", &task.path, Err(error))
                            .ok();
                    }
                }
            }
            Stage::Inspecting(receiver) => {
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => {
                        self.file.activity.viewer_export = Some(task);
                        return;
                    }
                    Err(error @ TryRecvError::Disconnected) => Err(ExportError::failed(error)),
                };
                if task.close_requested && result.is_ok() {
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                    context.request_repaint();
                    return;
                }
                let snapshot = self.document.current();
                let result = result.and_then(|(prepared, precondition)| {
                    if snapshot.document_id() != task.document
                        || snapshot.revision_id() != task.revision
                        || self.stored_viewer_settings().ok().as_ref() != Some(&task.settings)
                    {
                        return Err(ExportError::ChangedDuringConsent);
                    }
                    prepared.authorize(self, &task.path, &precondition)?;
                    Ok((prepared, precondition))
                });
                match result {
                    Ok((prepared, precondition)) => {
                        let worker_path = task.path.clone();
                        let repaint = context.clone();
                        let (sender, receiver) = mpsc::channel();
                        std::thread::spawn(move || {
                            if sender
                                .send(prepared.write(&worker_path, &precondition))
                                .is_ok()
                            {
                                repaint.request_repaint();
                            }
                        });
                        task.stage = Stage::Writing(receiver);
                        self.file.activity.viewer_export = Some(task);
                    }
                    Err(error) => {
                        self.report_export_outcome("viewer", &task.path, Err(error))
                            .ok();
                    }
                }
            }
            Stage::Writing(receiver) => {
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => {
                        self.file.activity.viewer_export = Some(task);
                        return;
                    }
                    Err(error @ TryRecvError::Disconnected) => Err(ExportError::failed(error)),
                };
                let written = self.report_export_outcome("viewer", &task.path, result);
                if written.is_ok() && task.close_requested {
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        context.request_repaint();
    }
}

#[cfg(test)]
mod tests;
