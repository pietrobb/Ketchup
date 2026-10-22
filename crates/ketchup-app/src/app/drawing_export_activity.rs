//! The project PDF and a drawing sheet's DXF or DWG from the drawings window:
//! the projection, hidden-line removal and file bytes are made on a thread, so
//! the window keeps painting and the file activity indication can cancel; the
//! file is written when they come back, so a cancelled export writes nothing
//! (review 2026-10-18 UI-11).

use super::drawing_sheet_export::{
    DrawingSheetExportError, ExportedDrawingSheet, SheetFileFormat, drawing_sheet_bytes,
    drawing_sheet_target, publish_drawing_sheet,
};
use super::project_drawings::{
    ProjectDrawingsError, project_drawings_target, publish_project_drawings,
};
use crate::*;
use ketchup_manufacturing::project_drawings::ProjectSheet;
use ketchup_model::drawing::DrawingSheetId;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// Which drawing is exported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DrawingExport {
    Project,
    Sheet(DrawingSheetId, SheetFileFormat),
}

/// What the thread made: the drawing and the bytes of its file.
pub(crate) enum MadeDrawing {
    Project(Result<(ProjectSheet, Vec<u8>), ProjectDrawingsError>),
    Sheet(Result<(ExportedDrawingSheet, Vec<u8>), DrawingSheetExportError>),
}

type DrawingWork = Box<dyn FnOnce() -> MadeDrawing + Send>;

pub(crate) struct DrawingExportTask {
    export: DrawingExport,
    path: PathBuf,
    /// The SHA-256 of the file the user chose to replace, taken when the
    /// export started; a file saved there meanwhile is not overwritten.
    replacing: Option<String>,
    receiver: Receiver<MadeDrawing>,
}

impl DrawingExportTask {
    /// The key of the window's button that started it.
    pub(crate) fn label_key(&self) -> &'static str {
        match self.export {
            DrawingExport::Project => "drawings-export-pdf",
            DrawingExport::Sheet(_, SheetFileFormat::Dxf) => "drawings-export-sheet-dxf",
            DrawingExport::Sheet(_, SheetFileFormat::Dwg) => "drawings-export-sheet-dwg",
        }
    }
}

impl KetchupApp {
    /// Takes what `export` is drawn from and starts it on a thread;
    /// [`Self::poll_drawing_export`] writes the file.
    pub(crate) fn start_drawing_export(
        &mut self,
        export: DrawingExport,
        path: PathBuf,
        context: &egui::Context,
    ) {
        let work: DrawingWork = match export {
            DrawingExport::Project => match self.project_drawings_job() {
                Ok(job) => Box::new(move || MadeDrawing::Project(job.render())),
                Err(error) => {
                    self.report_project_drawings(&path, &Err(error));
                    return;
                }
            },
            DrawingExport::Sheet(id, format) => {
                if self.exact.task.is_some() {
                    let error = Err(DrawingSheetExportError::Evaluating);
                    self.report_drawing_sheet(&path, format, &error);
                    return;
                }
                let snapshot = self.document.current();
                let results = self.exact.results.clone();
                Box::new(move || {
                    MadeDrawing::Sheet(drawing_sheet_bytes(&snapshot, &results, id, format))
                })
            }
        };
        self.queue_drawing_export(export, path, context, work);
    }

    pub(super) fn queue_drawing_export(
        &mut self,
        export: DrawingExport,
        path: PathBuf,
        context: &egui::Context,
        work: DrawingWork,
    ) {
        let replacing = match export {
            DrawingExport::Project => project_drawings_target(&path)
                .map_err(|error| self.report_project_drawings(&path, &Err(error))),
            DrawingExport::Sheet(_, format) => drawing_sheet_target(&path)
                .map_err(|error| self.report_drawing_sheet(&path, format, &Err(error))),
        };
        let Ok(replacing) = replacing else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let repaint = context.clone();
        std::thread::spawn(move || {
            if sender.send(work()).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.drawing_export = Some(DrawingExportTask {
            export,
            path,
            replacing,
            receiver,
        });
        context.request_repaint();
    }

    /// Drops the export; its thread finishes, but nothing is written.
    pub(super) fn cancel_drawing_export(&mut self) {
        if self.file.activity.drawing_export.take().is_some() {
            self.digest = self.catalog.text("digest-cancelled");
        }
    }

    pub(super) fn poll_drawing_export(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.drawing_export.as_ref() else {
            return;
        };
        let made = match task.receiver.try_recv() {
            Ok(made) => Some(made),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        };
        let task = self
            .file
            .activity
            .drawing_export
            .take()
            .expect("polled export");
        let ended = || std::io::Error::other("the drawing export ended unexpectedly");
        match (task.export, made) {
            (DrawingExport::Project, made) => {
                let result = match made {
                    Some(MadeDrawing::Project(result)) => result,
                    _ => Err(ProjectDrawingsError::Write(ended())),
                }
                .and_then(|(sheet, pdf)| {
                    publish_project_drawings(&task.path, &pdf, task.replacing.as_deref())
                        .map(|()| sheet)
                });
                self.report_project_drawings(&task.path, &result);
            }
            (DrawingExport::Sheet(_, format), made) => {
                let result = match made {
                    Some(MadeDrawing::Sheet(result)) => result,
                    _ => Err(DrawingSheetExportError::Write(
                        crate::export_bundle::ExportTransactionError::Io {
                            path: task.path.clone(),
                            cause: ended(),
                        },
                    )),
                }
                .and_then(|(exported, bytes)| {
                    publish_drawing_sheet(&task.path, &bytes, task.replacing.as_deref())
                        .map(|()| exported)
                });
                self.report_drawing_sheet(&task.path, format, &result);
            }
        }
        context.request_repaint();
    }
}
