//! A STEP or IGES file is read for its import review off the window's thread
//! (review 2026-10-19 UI-14). The exact worker's XDE inspection takes as long
//! as the file is large, so the window shows it with a Cancel that stops the
//! worker, as for a DWG file, and confirming the review publishes from what the
//! worker read instead of reading the file in XDE again.

use crate::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// What the exact worker read of a STEP or IGES file for its review.
#[derive(Clone, Debug)]
pub(crate) enum ExactImportEvidence {
    Step(StepXdeImportEvidence),
    Iges(IgesXdeImportEvidence),
}

/// The XDE evidence of `source`, the bytes of a `format` file, read by the
/// exact worker at `executable`; `cancelled` stops the worker.
pub(crate) fn inspect_exact_import(
    format: ImportFormat,
    executable: &Path,
    source: &[u8],
    cancelled: &AtomicBool,
) -> Result<ExactImportEvidence, ImportError> {
    let failed =
        |error: Box<dyn std::error::Error + Send + Sync>| ImportError::failed(format, error);
    let suffix = match format {
        ImportFormat::Iges => ".iges",
        _ => ".step",
    };
    let mut temporary = tempfile::Builder::new()
        .suffix(suffix)
        .tempfile()
        .map_err(|error| failed(error.into()))?;
    temporary
        .write_all(source)
        .and_then(|()| temporary.flush())
        .map_err(|error| failed(error.into()))?;
    let source_sha256 = sha256_bytes(source)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut worker = ExactWorkerSupervisor::spawn_with_cancellation(executable, cancelled)
        .map_err(|error| failed(error.into()))?;
    match format {
        ImportFormat::Iges => worker
            .inspect_iges_xde_import_with_cancellation(temporary.path(), &source_sha256, cancelled)
            .map(ExactImportEvidence::Iges),
        _ => worker
            .inspect_step_xde_import_with_cancellation(temporary.path(), &source_sha256, cancelled)
            .map(ExactImportEvidence::Step),
    }
    .map_err(|error| failed(error.into()))
}

/// A STEP or IGES file's bytes and the worker's evidence of them, or why they
/// could not be read.
pub(crate) type ExactImportRead = Result<(Vec<u8>, ExactImportEvidence), ImportError>;

/// A STEP or IGES file being read for its import review, off the window's thread.
pub(crate) struct ExactImportReadTask {
    format: ImportFormat,
    path: PathBuf,
    cancelled: Arc<AtomicBool>,
    receiver: std::sync::mpsc::Receiver<ExactImportRead>,
}

impl ExactImportReadTask {
    /// The menu command the window shows the read as.
    pub(crate) fn command(&self) -> AppCommand {
        match self.format {
            ImportFormat::Iges => AppCommand::ImportExactIges,
            _ => AppCommand::ImportExactStep,
        }
    }
}

impl KetchupApp {
    /// Reads the `format` file at `path` for its review: in a frame on a
    /// thread while the window shows it, else here.
    pub(crate) fn start_exact_import_read(&mut self, format: ImportFormat, path: PathBuf) {
        let executable = self.exact_worker_executable();
        let read = move |path: &Path, cancelled: &AtomicBool| -> ExactImportRead {
            let source = match format {
                ImportFormat::Iges => Self::read_iges_source(path)?,
                _ => Self::read_step_source(path)?,
            };
            let executable = executable.map_err(|error| ImportError::failed(format, error))?;
            let evidence = inspect_exact_import(format, &executable, &source, cancelled)?;
            Ok((source, evidence))
        };
        match self.file.activity.context.clone() {
            Some(context) => self.queue_exact_import_read(format, path, &context, read),
            None => {
                let read = read(&path, &AtomicBool::new(false));
                self.open_exact_import_review(format, path, read);
            }
        }
    }

    /// [`Self::start_exact_import_read`] on a thread, with the reader it runs.
    pub(crate) fn queue_exact_import_read(
        &mut self,
        format: ImportFormat,
        path: PathBuf,
        context: &egui::Context,
        read: impl FnOnce(&Path, &AtomicBool) -> ExactImportRead + Send + 'static,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (thread_path, thread_cancelled, repaint) =
            (path.clone(), Arc::clone(&cancelled), context.clone());
        std::thread::spawn(move || {
            if sender.send(read(&thread_path, &thread_cancelled)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.exact_import_reading = Some(ExactImportReadTask {
            format,
            path,
            cancelled,
            receiver,
        });
    }

    pub(super) fn poll_exact_import_read(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.exact_import_reading.as_ref() else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(ImportError::failed(
                task.format,
                "the thread that reads the file stopped",
            )),
        };
        let task = self
            .file
            .activity
            .exact_import_reading
            .take()
            .expect("polled task");
        self.open_exact_import_review(task.format, task.path, result);
        context.request_repaint();
    }

    /// Stops reading a STEP or IGES file; the worker reading it is stopped.
    pub(super) fn cancel_exact_import_read(&mut self) {
        if let Some(task) = self.file.activity.exact_import_reading.take() {
            task.cancelled.store(true, Ordering::Release);
        }
    }

    /// Opens the review of what `read` read of the file at `path`, sealed
    /// against the document as it is now, or says why it cannot.
    fn open_exact_import_review(
        &mut self,
        format: ImportFormat,
        path: PathBuf,
        read: ExactImportRead,
    ) {
        let review = read.and_then(|(source, evidence)| {
            let source =
                ImportSourcePlan::seal(format, path.clone(), source, (), &self.document.current());
            match evidence {
                ExactImportEvidence::Step(evidence) => {
                    let plan = self.step_import_preview_plan_from(source, evidence)?;
                    self.modal.open(PendingStepImport {
                        plan,
                        invalidated: false,
                    });
                }
                ExactImportEvidence::Iges(evidence) => {
                    let plan = self.iges_import_preview_plan_from(source, evidence)?;
                    self.modal.open(PendingIgesImport {
                        plan,
                        invalidated: false,
                    });
                }
            }
            Ok(())
        });
        if let Err(reason) = review {
            match format {
                ImportFormat::Iges => {
                    self.modal.close::<PendingIgesImport>();
                }
                _ => {
                    self.modal.close::<PendingStepImport>();
                }
            }
            self.report_import_failure(&path, &reason);
        }
    }
}
