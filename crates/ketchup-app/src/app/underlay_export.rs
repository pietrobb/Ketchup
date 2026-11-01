//! A drawing imported as an underlay written back as DXF with what was changed of it in
//! Kečup. The program worker reads the file it was imported from again (a DWG converted
//! to DXF) and writes it with the objects taken out left out, the moved ones where they
//! lie and the added ones after the drawing's own; layers, styles, blocks, dimensions,
//! hatches and every object not changed stay as the file had them.
//!
//! The worker writes into a folder of its own; the drawing then takes the place of the
//! file named by a rename, only if that file is still what the user agreed to replace,
//! and never of the file the drawing is read from (review 2026-10-29 SEC-60). A worker
//! stopped half way leaves every file named as it was. The window's export runs on a
//! thread its file activity indication cancels, and the worker has
//! [`UNDERLAY_EXPORT_TIME`] for it.

use crate::export_bundle::{ExportTransactionError, write_export_artifact_if_unchanged};
use crate::*;
use ketchup_application::program_isolation::{
    UnderlayWorkerError, UnderlayWriteInput, UnderlayWritten, write_underlay,
};
use ketchup_model::underlay::UnderlayId;
use ketchup_program::CancelToken;

/// The time the worker has to write a drawing back. Levoča, a 38.9 MB DWG, is read and
/// converted in 4.1 s in a release build; writing it back reads it again and converts the
/// result once more, so this leaves room for a drawing many times its size.
pub(crate) const UNDERLAY_EXPORT_TIME: Duration = Duration::from_secs(300);

/// Why a drawing was not written back.
#[derive(Debug)]
pub(crate) enum UnderlayExportError {
    /// The document has no such underlay.
    NotFound(UnderlayId),
    /// The drawing was not imported in this window and no source file was given.
    NoSource(UnderlayId),
    /// The document holds no blob of its lines.
    NoLines(UnderlayId),
    /// The worker's files could not be written or read.
    Io(std::io::Error),
    Worker(UnderlayWorkerError),
    /// The file to write is the file the drawing is read from.
    SameAsSource(PathBuf),
    /// The drawing written did not take the place of the file named.
    Publish(ExportTransactionError),
}

impl std::fmt::Display for UnderlayExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(formatter, "the document has no underlay {}", id.0),
            Self::NoSource(id) => write!(
                formatter,
                "underlay {} was not imported in this window; name the DXF or DWG file it \
                 was imported from",
                id.0
            ),
            Self::NoLines(id) => write!(
                formatter,
                "the document holds no lines of underlay {}; import it again",
                id.0
            ),
            Self::Io(error) => write!(formatter, "the drawing was not written: {error}"),
            Self::Worker(error) => error.fmt(formatter),
            Self::SameAsSource(path) => write!(
                formatter,
                "{} is the file the drawing is read from; export it to another file",
                path.display()
            ),
            Self::Publish(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for UnderlayExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Worker(error) => Some(error),
            Self::Publish(error) => Some(error),
            _ => None,
        }
    }
}

type UnderlayExportResult = Result<UnderlayWritten, UnderlayExportError>;

/// A drawing to write back, taken from the document so it is written off the window's
/// thread.
pub(crate) struct UnderlayExportJob {
    source: PathBuf,
    lines: Vec<u8>,
    /// How many layers the underlay names.
    layers: usize,
    edits: ketchup_model::underlay::edits::UnderlayEdits,
    /// What its lines were moved by when read.
    offset_mm: [f64; 2],
    output: PathBuf,
    replacing: Option<String>,
}

/// A drawing the worker wrote, not yet in place of the file named.
pub(crate) struct WrittenDrawing {
    output: PathBuf,
    replacing: Option<String>,
    bytes: Vec<u8>,
    /// The records of a DWG source its read left out, by type.
    skipped: Vec<(String, usize)>,
}

impl WrittenDrawing {
    /// Puts the drawing in place of the file named; gives its length and what it lacks.
    fn publish(self) -> UnderlayExportResult {
        write_export_artifact_if_unchanged(&self.output, &self.bytes, self.replacing.as_deref())
            .map_err(UnderlayExportError::Publish)?;
        Ok(UnderlayWritten {
            bytes: u64::try_from(self.bytes.len()).unwrap_or(u64::MAX),
            skipped: self.skipped,
        })
    }
}

impl UnderlayExportJob {
    /// Writes the drawing in the worker, then puts it in place of `output`; gives the
    /// bytes written.
    fn run(self, cancel: &CancelToken) -> UnderlayExportResult {
        self.write(cancel)?.publish()
    }

    /// Writes the drawing in the worker, into a folder of its own.
    fn write(self, cancel: &CancelToken) -> Result<WrittenDrawing, UnderlayExportError> {
        let directory = tempfile::tempdir().map_err(UnderlayExportError::Io)?;
        let lines = directory.path().join("underlay-lines.bin");
        std::fs::write(&lines, &self.lines).map_err(UnderlayExportError::Io)?;
        // The worker writes DWG to a `.dwg` output, DXF otherwise.
        let to_dwg = crate::app::dwg_import::is_dwg_path(&self.output);
        let written = directory
            .path()
            .join(if to_dwg { "drawing.dwg" } else { "drawing.dxf" });
        let worker = write_underlay(
            UnderlayWriteInput {
                dwg: crate::app::dwg_import::is_dwg_path(&self.source),
                source: self.source,
                // A drawing that declares no unit is matched against its lines in each.
                unit_if_unitless: Some(ImportLengthUnit::Millimetre),
                // Read again it moves as it moved then, also by the document's geo origin.
                geo_origin_mm: (self.offset_mm != [0.0; 2]).then_some(self.offset_mm),
                lines,
                layers: self.layers,
                edits: self.edits,
                output: written.clone(),
            },
            UNDERLAY_EXPORT_TIME,
            cancel,
        )
        .map_err(UnderlayExportError::Worker)?;
        Ok(WrittenDrawing {
            bytes: std::fs::read(&written).map_err(UnderlayExportError::Io)?,
            output: self.output,
            replacing: self.replacing,
            skipped: worker.skipped,
        })
    }
}

/// The window's drawing export, running on a thread.
pub(crate) struct UnderlayExportTask {
    id: UnderlayId,
    output: PathBuf,
    /// The source file the user named, kept for the next export once this one is written.
    named_source: Option<PathBuf>,
    pub(crate) dwg: bool,
    cancel: CancelToken,
    /// The drawing written; it replaces the file named when it comes back, so a cancelled
    /// export replaces nothing.
    receiver: mpsc::Receiver<Result<WrittenDrawing, UnderlayExportError>>,
}

/// A client's drawing export running on a thread (review 2026-10-31 MCP-80): the
/// drawing the worker wrote comes back on `receiver`; `cancel` stops the worker.
pub(crate) struct ClientUnderlayExport {
    pub(crate) cancel: CancelToken,
    pub(crate) receiver: mpsc::Receiver<Result<WrittenDrawing, UnderlayExportError>>,
}

impl ClientUnderlayExport {
    /// Puts the drawing written in place of the file named.
    pub(crate) fn publish(
        written: Result<WrittenDrawing, UnderlayExportError>,
    ) -> UnderlayExportResult {
        written.and_then(WrittenDrawing::publish)
    }
}

impl KetchupApp {
    /// Starts writing underlay `id` with its changes to `output` on a thread, as
    /// Tools > Export drawing writes it, so the window keeps drawing; no file is
    /// in place until [`ClientUnderlayExport::publish`].
    pub(crate) fn start_client_underlay_export(
        &self,
        id: UnderlayId,
        source: Option<&Path>,
        output: &Path,
        context: &egui::Context,
    ) -> Result<ClientUnderlayExport, UnderlayExportError> {
        let job = self.underlay_export_job(id, source, output, None)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = CancelToken::default();
        let thread_cancel = cancel.clone();
        let repaint = context.clone();
        std::thread::Builder::new()
            .name("ketchup-drawing-export".into())
            .spawn(move || {
                let _ = sender.try_send(job.write(&thread_cancel));
                repaint.request_repaint();
            })
            .map_err(UnderlayExportError::Io)?;
        Ok(ClientUnderlayExport { cancel, receiver })
    }

    /// What writing underlay `id` with its changes to `output` takes, read again from
    /// `source` or else from the file it was imported from in this window. A file at
    /// `output` is to be replaced only while its SHA-256 is `replacing` (`None`: no file
    /// may be there), and never when it is the source.
    fn underlay_export_job(
        &self,
        id: UnderlayId,
        source: Option<&Path>,
        output: &Path,
        replacing: Option<&str>,
    ) -> Result<UnderlayExportJob, UnderlayExportError> {
        let snapshot = self.document.current();
        let underlay = snapshot
            .underlay(id)
            .ok_or(UnderlayExportError::NotFound(id))?;
        let source = source
            .map(Path::to_owned)
            .or_else(|| self.file.activity.underlay.sources.get(&id).cloned())
            .ok_or(UnderlayExportError::NoSource(id))?;
        if let (Ok(read), Ok(written)) = (source.canonicalize(), output.canonicalize())
            && read == written
        {
            return Err(UnderlayExportError::SameAsSource(output.to_owned()));
        }
        let blobs = self.file.container_data.blobs();
        let lines = blobs
            .get(&underlay.lines_blob())
            .ok_or(UnderlayExportError::NoLines(id))?
            .as_slice()
            .to_vec();
        Ok(UnderlayExportJob {
            source,
            lines,
            layers: underlay.layers.len(),
            edits: underlay.edits.clone(),
            offset_mm: underlay.offset_mm(),
            output: output.to_owned(),
            replacing: replacing.map(str::to_owned),
        })
    }

    /// Writes underlay `id` with its changes to `output` as DXF (DWG for a `.dwg`), read
    /// again from `source`
    /// or else from the file it was imported from in this window; gives the bytes written.
    /// A file at `output` is replaced only while its SHA-256 is `replacing` (`None`: no
    /// file may be there), and never when it is the source; `cancel` stops the worker.
    #[cfg(test)]
    pub(crate) fn export_underlay_dxf_to(
        &mut self,
        id: UnderlayId,
        source: Option<&Path>,
        output: &Path,
        replacing: Option<&str>,
        cancel: &CancelToken,
    ) -> UnderlayExportResult {
        self.underlay_export_job(id, source, output, replacing)?
            .run(cancel)
    }

    /// Tools > Export drawing to DXF or DWG: the selected drawing, with its changes, to a
    /// file of `extension` the user names; the file it was imported from is asked for when
    /// this window did not import it or it is no longer there. It is written on a thread
    /// while a frame shows it, or here in a harness without one.
    pub(crate) fn export_selected_underlay(&mut self, extension: &str) {
        let Some(id) = self.selected_underlay() else {
            return;
        };
        let known = self.file.activity.underlay.sources.get(&id);
        let named_source = if known.is_some_and(|path| path.is_file()) {
            None
        } else {
            match self.choose_dxf_import_path() {
                Some(path) => Some(path),
                None => return,
            }
        };
        let Some(output) = self.choose_export_path(extension) else {
            return;
        };
        // The file the user agreed to replace in the dialog, as it is now.
        let job = crate::export_bundle::export_target_sha256(&output)
            .map_err(UnderlayExportError::Publish)
            .and_then(|replacing| {
                self.underlay_export_job(id, named_source.as_deref(), &output, replacing.as_deref())
            });
        let job = match job {
            Ok(job) => job,
            Err(error) => return self.finish_underlay_export(id, &output, None, Err(error)),
        };
        let Some(context) = self.file.activity.context.clone() else {
            let result = job.run(&CancelToken::default());
            return self.finish_underlay_export(id, &output, named_source, result);
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = CancelToken::default();
        let thread_cancel = cancel.clone();
        std::thread::spawn(move || {
            if sender.send(job.write(&thread_cancel)).is_ok() {
                context.request_repaint();
            }
        });
        self.file.activity.underlay.exporting = Some(UnderlayExportTask {
            id,
            dwg: crate::app::dwg_import::is_dwg_path(&output),
            output,
            named_source,
            cancel,
            receiver,
        });
    }

    pub(crate) fn poll_underlay_export(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.underlay.exporting.as_ref() else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(written) => written.and_then(WrittenDrawing::publish),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(UnderlayExportError::Io(std::io::Error::other(
                "the thread that writes the drawing stopped",
            ))),
        };
        let task = self
            .file
            .activity
            .underlay
            .exporting
            .take()
            .expect("polled task");
        self.finish_underlay_export(task.id, &task.output, task.named_source, result);
        context.request_repaint();
    }

    /// Stops writing a drawing back: the worker is killed and no file is replaced.
    pub(crate) fn cancel_underlay_export(&mut self) {
        if let Some(task) = self.file.activity.underlay.exporting.take() {
            task.cancel.cancel();
            self.digest = self.catalog.text("digest-cancelled");
        }
    }

    fn finish_underlay_export(
        &mut self,
        id: UnderlayId,
        output: &Path,
        named_source: Option<PathBuf>,
        result: UnderlayExportResult,
    ) {
        self.digest = match &result {
            Ok(written) if written.skipped.is_empty() => self.catalog.format(
                "digest-underlay-exported",
                &BTreeMap::from([("path", output.display().to_string())]),
            ),
            // What the DWG reader could not read is said, not left out silently.
            Ok(written) => self.catalog.format(
                "digest-underlay-exported-incomplete",
                &BTreeMap::from([
                    ("path", output.display().to_string()),
                    (
                        "count",
                        written
                            .skipped
                            .iter()
                            .map(|(_, count)| count)
                            .sum::<usize>()
                            .to_string(),
                    ),
                ]),
            ),
            Err(error) => self.catalog.format(
                "digest-underlay-export-failed",
                &BTreeMap::from([("reason", error.to_string())]),
            ),
        };
        // The file named is the drawing's source: the next export does not ask again.
        if let (Ok(_), Some(source)) = (result, named_source) {
            self.file.activity.underlay.sources.insert(id, source);
        }
    }
}
