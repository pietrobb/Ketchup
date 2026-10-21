//! A DXF or DWG drawing imported as an underlay: read in the program worker on a thread
//! while the window shows that it is reading (its Cancel kills the worker, as does the
//! worker's DWG deadline), then added as one Undo step with its lines as a blob of the document. The
//! viewport draws the lines of every visible layer under the model.

use crate::import_source::BoxedCause;
use crate::renderer::underlay::{UnderlayDraw, UnderlayGeometry, UnderlayPaintCallback};
use crate::*;
use ketchup_application::program_isolation::{
    UnderlayInput, UnderlayRead, UnderlayReadError, UnderlayRefusal, UnderlayWorkerError,
    read_underlay,
};
use ketchup_model::underlay::{
    UnderlayId, UnderlayLines, UnderlayPlanError, plan_underlay, plan_underlay_removal,
};
use ketchup_program::CancelToken;

/// A drawing read as an underlay, not yet added to the document.
pub(crate) struct ReadUnderlay {
    pub(crate) read: UnderlayRead,
    /// The encoded lines, the blob the document keeps.
    bytes: Vec<u8>,
    lines: UnderlayLines,
}

impl ReadUnderlay {
    /// What was read, for a bridge client: layers, lines and the first diagnostics.
    pub(crate) fn summary(&self) -> serde_json::Value {
        const LISTED: usize = 20;
        let read = &self.read;
        serde_json::json!({
            "layers": read.layers.iter().take(LISTED).collect::<Vec<_>>(),
            "layers_total": read.layers.len(),
            "lines": read.segments,
            "polylines": self.lines.layers().iter().map(|layer| layer.polylines().count()).sum::<usize>(),
            "units": read.units,
            "offset_mm": read.offset_mm,
            "diagnostics": read.diagnostics.iter().take(LISTED).collect::<Vec<_>>(),
            "diagnostics_total": read.diagnostics.len(),
            "dwg_version": read.dwg.as_ref().map(|(version, _)| version),
        })
    }
}

/// Reads the drawing at `path` in the program worker; `unit` is the length unit of a
/// drawing that declares none.
pub(crate) fn read_underlay_file(
    path: &Path,
    unit: Option<ImportLengthUnit>,
    cancel: &CancelToken,
) -> Result<ReadUnderlay, ImportError> {
    let failed = |cause: BoxedCause| ImportError::failed(ImportFormat::Dxf, cause);
    let directory = tempfile::tempdir().map_err(|error| failed(error.into()))?;
    let output = directory.path().join("underlay-lines.bin");
    let read = read_underlay(
        UnderlayInput {
            source: path.to_owned(),
            dwg: crate::app::dwg_import::is_dwg_path(path),
            unit_if_unitless: unit,
            output: output.clone(),
        },
        cancel,
    )
    .map_err(|error| failed(error.into()))?;
    let bytes = std::fs::read(&output).map_err(|error| failed(error.into()))?;
    let lines = UnderlayLines::decode(&bytes).map_err(|error| failed(error.into()))?;
    Ok(ReadUnderlay { read, bytes, lines })
}

/// Whether `error` refused a drawing only because it declares no length unit.
fn needs_unit(error: &ImportError) -> bool {
    matches!(&error.failure, ImportFailure::Failed(cause) if matches!(
        cause.downcast_ref::<UnderlayWorkerError>(),
        Some(UnderlayWorkerError::Read(UnderlayReadError::Refused(
            UnderlayRefusal::UnitsRequired
        )))
    ))
}

pub(crate) type UnderlayReadResult = Result<ReadUnderlay, ImportError>;

/// Why an underlay was not removed.
#[derive(Debug)]
pub(crate) enum UnderlayRemovalError {
    Plan(UnderlayPlanError),
    Commit(WorkRecoveryMutationError<CanonicalError>),
}

impl std::fmt::Display for UnderlayRemovalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(formatter),
            Self::Commit(error) => write!(formatter, "the removal was not kept: {error}"),
        }
    }
}

impl std::error::Error for UnderlayRemovalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Plan(error) => Some(error),
            Self::Commit(error) => Some(error),
        }
    }
}

/// A drawing being read as an underlay for the window, off its thread.
pub(crate) struct UnderlayReadTask {
    path: PathBuf,
    unit: Option<ImportLengthUnit>,
    cancel: CancelToken,
    receiver: mpsc::Receiver<UnderlayReadResult>,
    pub(crate) progress: super::file_activity::ReadProgress,
}

/// The window's underlay import: the read in progress, or the drawing that waits for the
/// user to choose its unit.
#[derive(Default)]
pub(crate) struct UnderlayImport {
    pub(crate) reading: Option<UnderlayReadTask>,
    unit_choice: Option<(PathBuf, ImportLengthUnit)>,
}

impl KetchupApp {
    /// Reads `path` as an underlay: on a thread while a frame shows it, or here in a
    /// harness without one.
    pub(crate) fn start_underlay_read(&mut self, path: PathBuf, unit: Option<ImportLengthUnit>) {
        let Some(context) = self.file.activity.context.clone() else {
            let result = read_underlay_file(&path, unit, &CancelToken::default());
            self.finish_underlay_read(path, unit, result);
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = CancelToken::default();
        let (thread_path, thread_cancel) = (path.clone(), cancel.clone());
        std::thread::spawn(move || {
            if sender
                .send(read_underlay_file(&thread_path, unit, &thread_cancel))
                .is_ok()
            {
                context.request_repaint();
            }
        });
        self.file.activity.underlay.reading = Some(UnderlayReadTask {
            progress: super::file_activity::ReadProgress::start(&path),
            path,
            unit,
            cancel,
            receiver,
        });
    }

    pub(crate) fn poll_underlay_read(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.underlay.reading.as_ref() else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ImportError::failed(
                ImportFormat::Dxf,
                "the thread that reads the drawing stopped",
            )),
        };
        let task = self
            .file
            .activity
            .underlay
            .reading
            .take()
            .expect("polled task");
        self.finish_underlay_read(task.path, task.unit, result);
        context.request_repaint();
    }

    /// Stops reading a drawing; the worker reading it is killed.
    pub(crate) fn cancel_underlay_read(&mut self) {
        if let Some(task) = self.file.activity.underlay.reading.take() {
            task.cancel.cancel();
        }
    }

    fn finish_underlay_read(
        &mut self,
        path: PathBuf,
        unit: Option<ImportLengthUnit>,
        result: UnderlayReadResult,
    ) {
        match result.and_then(|read| self.add_underlay(&path, read)) {
            Ok(_) => {}
            Err(error) if unit.is_none() && needs_unit(&error) => {
                self.file.activity.underlay.unit_choice =
                    Some((path, ImportLengthUnit::Millimetre));
            }
            Err(error) => self.report_import_failure(&path, &error),
        }
    }

    /// Adds the drawing read from `path` as an underlay named after the file, as one Undo
    /// step, and reports it.
    pub(crate) fn add_underlay(
        &mut self,
        path: &Path,
        read: ReadUnderlay,
    ) -> Result<UnderlayId, ImportError> {
        let failed = |cause: BoxedCause| ImportError::failed(ImportFormat::Dxf, cause);
        let name = path
            .file_stem()
            .map_or_else(|| "underlay".into(), |stem| stem.to_string_lossy());
        let key = sha256_bytes(&read.bytes);
        let (batch, id) = plan_underlay(
            &self.document.current(),
            &name,
            &read.read.layers,
            key,
            read.read.offset_mm,
        )
        .map_err(|error| failed(error.into()))?;
        let mut container = self.file.container_data.clone();
        container
            .insert_import_blob(read.bytes)
            .map_err(|error| failed(error.into()))?;
        let proposal = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
            .map_err(|error| failed(error.into()))?;
        self.commit_verified_proposal_with_container_work_recovery(&proposal, container)
            .map_err(|error| failed(error.into()))?;
        self.render
            .underlays
            .insert(key, Arc::new(UnderlayGeometry::new(key, &read.lines)));
        self.digest = self.catalog.format(
            "digest-imported-underlay",
            &BTreeMap::from([
                ("path", path.display().to_string()),
                ("layers", read.read.layers.len().to_string()),
                ("lines", read.read.segments.to_string()),
            ]),
        );
        Ok(id)
    }

    /// Removes the underlay `id` and the tags only it held, as one Undo step.
    pub(crate) fn remove_underlay(&mut self, id: UnderlayId) -> Result<(), UnderlayRemovalError> {
        let batch = plan_underlay_removal(&self.document.current(), id)
            .map_err(UnderlayRemovalError::Plan)?;
        self.apply_batch_with_work_recovery(&batch)
            .map_err(UnderlayRemovalError::Commit)
    }

    /// Asks for the unit of a drawing that declares none, then reads it again.
    pub(crate) fn show_underlay_unit_window(&mut self, context: &egui::Context) {
        let Some((path, mut unit)) = self.file.activity.underlay.unit_choice.clone() else {
            return;
        };
        let (mut import, mut cancel) = (false, false);
        egui::Window::new(self.catalog.text("dialog-underlay-unit-title"))
            .id(egui::Id::new("underlay-unit"))
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-underlay-unit-text",
                    &BTreeMap::from([("path", path.display().to_string())]),
                ));
                for (candidate, key) in [
                    (ImportLengthUnit::Millimetre, "unit-millimetre"),
                    (ImportLengthUnit::Centimetre, "unit-centimetre"),
                    (ImportLengthUnit::Metre, "unit-metre"),
                    (ImportLengthUnit::Inch, "unit-inch"),
                    (ImportLengthUnit::Foot, "unit-foot"),
                ] {
                    ui.radio_value(&mut unit, candidate, self.catalog.text(key));
                }
                ui.horizontal(|ui| {
                    import = ui
                        .button(self.catalog.text("dialog-underlay-unit-confirm"))
                        .clicked();
                    cancel = ui.button(self.catalog.text("action-cancel")).clicked();
                });
            });
        self.file.activity.underlay.unit_choice = if import || cancel {
            None
        } else {
            Some((path.clone(), unit))
        };
        if import {
            self.start_underlay_read(path, Some(unit));
        } else if cancel {
            self.digest = self.catalog.text("digest-cancelled");
        }
    }

    /// The underlays the viewport paints this frame ([`Self::paint_underlays`]).
    pub(crate) fn prepare_underlay_frame(&mut self) {
        self.render.underlay_frame = if self.render.wgpu_target_format.is_some() {
            self.underlay_draws()
        } else {
            Vec::new()
        };
    }

    /// Paints the visible layers of every underlay, over the ground grid and under the
    /// model.
    pub(crate) fn paint_underlays(&self, painter: &egui::Painter, rect: Rect) {
        if self.render.underlay_frame.is_empty() {
            return;
        }
        let dim = self.palette().dim;
        painter.add(eframe::egui_wgpu::Callback::new_paint_callback(
            rect,
            UnderlayPaintCallback::new(
                self.render.underlay_frame.clone(),
                self.world_to_clip(rect),
                [dim.r(), dim.g(), dim.b()],
            ),
        ));
    }

    /// Every underlay of the document with the vertex ranges of its visible layers. Lines
    /// are decoded from the document's blob once and kept while an underlay names them.
    pub(crate) fn underlay_draws(&mut self) -> Vec<UnderlayDraw> {
        let snapshot = self.document.current();
        let shown = |tag| snapshot.tag(tag).is_some_and(|tag| tag.visible());
        let mut draws = Vec::new();
        for underlay in snapshot.underlays() {
            let key = underlay.lines_sha256;
            let geometry = if let Some(geometry) = self.render.underlays.get(&key) {
                Arc::clone(geometry)
            } else {
                let blob = underlay.lines_blob();
                let decoded = self
                    .file
                    .container_data
                    .blobs()
                    .get(&blob)
                    .map(|bytes| UnderlayLines::decode(bytes));
                let reason = match decoded {
                    Some(Ok(lines)) => Ok(lines),
                    Some(Err(error)) => Err(error.to_string()),
                    None => Err(format!("the document holds no blob {blob}")),
                };
                // Shown, not drawn: the document's other content stays usable.
                let lines = reason.unwrap_or_else(|reason| {
                    self.digest = self.catalog.format(
                        "error-underlay-lines",
                        &BTreeMap::from([("name", underlay.name.clone()), ("reason", reason)]),
                    );
                    UnderlayLines::default()
                });
                let geometry = Arc::new(UnderlayGeometry::new(key, &lines));
                self.render.underlays.insert(key, Arc::clone(&geometry));
                geometry
            };
            let visible = if shown(underlay.tag) {
                underlay
                    .layers
                    .iter()
                    .zip(geometry.layers())
                    .filter(|(layer, _)| shown(layer.tag))
                    .map(|(_, range)| range.clone())
                    .collect()
            } else {
                Vec::new()
            };
            draws.push(UnderlayDraw { geometry, visible });
        }
        self.render.underlays.retain(|key, _| {
            snapshot
                .underlays()
                .any(|underlay| underlay.lines_sha256 == *key)
        });
        draws
    }
}
