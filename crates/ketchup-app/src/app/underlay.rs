//! A DXF or DWG drawing imported as an underlay: read in the program worker on a thread
//! while the window shows that it is reading (its Cancel kills the worker; a DWG read has no
//! deadline and otherwise stops only at the worker's memory limit), then added as one Undo
//! step with its lines as a blob of the document. The
//! viewport draws the lines of every visible layer under the model.

use super::underlay_snapping::UnderlaySnapSlot;
use crate::import_source::BoxedCause;
use crate::renderer::underlay::{UnderlayDraw, UnderlayGeometry, UnderlayPaintCallback};
use crate::*;
use ketchup_application::program_isolation::{
    UnderlayInput, UnderlayRead, UnderlayReadError, UnderlayRefusal, UnderlayWorkerError,
    read_underlay,
};
use ketchup_model::underlay::object_query::ObjectTree;
use ketchup_model::underlay::placement::{
    UnderlayFit, UnderlayFitError, geo_not_beside, geo_origin, plan_underlay_fit,
};
use ketchup_model::underlay::snap::UnderlaySnapIndex;
use ketchup_model::underlay::{
    LayerLines, Underlay, UnderlayId, UnderlayLines, UnderlayLinesError, UnderlayPlanError,
    plan_underlay, plan_underlay_removal,
};
use ketchup_program::CancelToken;

#[cfg(test)]
thread_local! {
    /// How many times this thread decoded an underlay's blob.
    pub(crate) static BLOB_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many times this thread worked out the object tree of an underlay's lines.
    pub(crate) static OBJECT_TREES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What one layer of an underlay's lines draws: the box of its points and text anchors in
/// their own millimetres (`None` for a layer that draws nothing), its lines and its texts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerExtent {
    bounds_mm: Option<[[f64; 2]; 2]>,
    lines: usize,
    texts: usize,
}

/// One [`LayerExtent`] per layer of an underlay's lines.
pub(crate) type UnderlayExtent = Arc<[LayerExtent]>;

/// A layer's lines are the entities that draw lines on it, an arc or a polyline one each
/// however many straight pieces draw it (review 2026-10-24 UI-30); an underlay saved
/// before its objects were kept counts its straight pieces.
fn underlay_extent(lines: &UnderlayLines) -> UnderlayExtent {
    let objects = &lines.objects().objects;
    let mut drawn = vec![0_usize; lines.layers().len()];
    for object in objects {
        let mut layers = object
            .parts
            .iter()
            .filter(|part| !part.points.is_empty())
            .map(|part| part.layer)
            .collect::<Vec<_>>();
        layers.sort_unstable();
        layers.dedup();
        for layer in layers {
            if let Some(count) = usize::try_from(layer)
                .ok()
                .and_then(|layer| drawn.get_mut(layer))
            {
                *count += 1;
            }
        }
    }
    lines
        .layers()
        .iter()
        .zip(drawn)
        .map(|(layer, drawn): (&LayerLines, usize)| LayerExtent {
            bounds_mm: layer.bounds_mm(),
            lines: if objects.is_empty() {
                layer.segment_count()
            } else {
                drawn
            },
            texts: layer.texts().len(),
        })
        .collect()
}

/// A drawing read as an underlay, not yet added to the document.
pub(crate) struct ReadUnderlay {
    pub(crate) read: UnderlayRead,
    /// The encoded lines, the blob the document keeps.
    bytes: Vec<u8>,
    lines: UnderlayLines,
    /// The snap index of `lines`, built on the reading thread: the window does not wait for it.
    snaps: Arc<UnderlaySnapIndex>,
    /// The GPU lines of `lines` and of the letters of its texts, laid out on the reading
    /// thread too.
    geometry: Arc<UnderlayGeometry>,
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
            "texts": read.texts,
            "fills": self.lines.fill_count(),
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
/// drawing that declares none, `geo_origin_mm` the document's
/// ([`ketchup_model::underlay::placement::geo_origin`]).
pub(crate) fn read_underlay_file(
    path: &Path,
    unit: Option<ImportLengthUnit>,
    geo_origin_mm: Option<[f64; 2]>,
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
            geo_origin_mm,
            output: output.clone(),
        },
        cancel,
    )
    .map_err(|error| failed(error.into()))?;
    let bytes = std::fs::read(&output).map_err(|error| failed(error.into()))?;
    let lines =
        UnderlayLines::decode(&bytes, read.layers.len()).map_err(|error| failed(error.into()))?;
    let snaps = Arc::new(UnderlaySnapIndex::new(&lines));
    let geometry = Arc::new(UnderlayGeometry::new(sha256_bytes(&bytes), &lines));
    Ok(ReadUnderlay {
        read,
        bytes,
        lines,
        snaps,
        geometry,
    })
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

/// Why an underlay was not removed or placed.
#[derive(Debug)]
pub(crate) enum UnderlayEditError {
    Plan(UnderlayPlanError),
    Fit(UnderlayFitError),
    Commit(WorkRecoveryMutationError<CanonicalError>),
}

impl std::fmt::Display for UnderlayEditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(formatter),
            Self::Fit(error) => error.fmt(formatter),
            Self::Commit(error) => write!(formatter, "the change was not kept: {error}"),
        }
    }
}

impl std::error::Error for UnderlayEditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Plan(error) => Some(error),
            Self::Fit(error) => Some(error),
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
    /// A drawing being written back with its changes.
    pub(crate) exporting: Option<super::underlay_export::UnderlayExportTask>,
    unit_choice: Option<(PathBuf, ImportLengthUnit)>,
    /// The file each underlay imported in this window was read from, so its drawing can
    /// be written back with its changes.
    pub(crate) sources: std::collections::HashMap<UnderlayId, PathBuf>,
}

impl KetchupApp {
    /// Reads `path` as an underlay: on a thread while a frame shows it, or here in a
    /// harness without one.
    pub(crate) fn start_underlay_read(&mut self, path: PathBuf, unit: Option<ImportLengthUnit>) {
        let origin = geo_origin(&self.document.current());
        let Some(context) = self.file.activity.context.clone() else {
            let result = read_underlay_file(&path, unit, origin, &CancelToken::default());
            self.finish_underlay_read(path, unit, result);
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = CancelToken::default();
        let (thread_path, thread_cancel) = (path.clone(), cancel.clone());
        std::thread::spawn(move || {
            if sender
                .send(read_underlay_file(
                    &thread_path,
                    unit,
                    origin,
                    &thread_cancel,
                ))
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
        let not_beside = geo_not_beside(&self.document.current(), read.read.offset_mm);
        let (batch, id) = plan_underlay(
            &self.document.current(),
            &name,
            &read.read.layers,
            &read.read.hidden_layers,
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
        self.render.underlays.insert(key, read.geometry);
        self.render
            .underlay_snaps
            .get_mut()
            .insert(key, UnderlaySnapSlot::Ready(read.snaps));
        self.render
            .underlay_extents
            .get_mut()
            .insert(key, underlay_extent(&read.lines));
        self.frame_after_import();
        self.file
            .activity
            .underlay
            .sources
            .insert(id, path.to_owned());
        self.digest = self.catalog.format(
            "digest-imported-underlay",
            &BTreeMap::from([
                ("path", path.display().to_string()),
                ("layers", read.read.layers.len().to_string()),
                ("lines", read.read.segments.to_string()),
                ("texts", read.read.texts.to_string()),
            ]),
        );
        if let Some(not_beside) = not_beside {
            let [x, y] = not_beside.apart_mm;
            let note = self.catalog.format(
                "digest-imported-underlay-not-beside",
                &BTreeMap::from([
                    ("first", not_beside.underlay_id.0.to_string()),
                    ("km", format!("{:.1}", x.hypot(y) / 1_000_000.0)),
                ]),
            );
            self.digest = format!("{} {note}", self.digest);
        }
        Ok(id)
    }

    /// Removes the underlay `id` and the tags only it held, as one Undo step.
    pub(crate) fn remove_underlay(&mut self, id: UnderlayId) -> Result<(), UnderlayEditError> {
        let batch =
            plan_underlay_removal(&self.document.current(), id).map_err(UnderlayEditError::Plan)?;
        self.apply_batch_with_work_recovery(&batch)
            .map_err(UnderlayEditError::Commit)
    }

    /// Places the underlay `id` so that the document points `from_mm`, two points of its
    /// lines where they are drawn now, land on the model points `to_mm`, scaled only when
    /// `scaled` ([`plan_underlay_fit`]), as one Undo step.
    pub(crate) fn fit_underlay(
        &mut self,
        id: UnderlayId,
        from_mm: [[f64; 2]; 2],
        to_mm: [[f64; 2]; 2],
        scaled: bool,
    ) -> Result<UnderlayFit, UnderlayEditError> {
        let (batch, fit) = plan_underlay_fit(&self.document.current(), id, from_mm, to_mm, scaled)
            .map_err(UnderlayEditError::Fit)?;
        self.apply_batch_with_work_recovery(&batch)
            .map_err(UnderlayEditError::Commit)?;
        Ok(fit)
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
        let pixels = rect.size() * painter.ctx().pixels_per_point();
        painter.add(eframe::egui_wgpu::Callback::new_paint_callback(
            rect,
            UnderlayPaintCallback::new(
                self.render.underlay_frame.clone(),
                (self.world_to_clip_f64(rect), self.view_ray_mm(rect)),
                [pixels.x, pixels.y],
                [dim.r(), dim.g(), dim.b()],
            ),
        ));
    }

    /// Every underlay of the document with the indices of its visible layers. Lines
    /// are decoded from the document's blob once and kept while an underlay names them.
    pub(crate) fn underlay_draws(&mut self) -> Vec<UnderlayDraw> {
        let snapshot = self.document.current();
        let shown = |tag| snapshot.tag(tag).is_some_and(|tag| tag.visible());
        let dragged = self.underlay_drag_edit();
        let mut draws = Vec::new();
        for underlay in snapshot.underlays() {
            let key = underlay.lines_key();
            let geometry = if let Some(geometry) = self.render.underlays.get(&key) {
                Arc::clone(geometry)
            } else {
                // Shown, not drawn: the document's other content stays usable.
                let lines = self
                    .underlay_lines_or_reason(underlay)
                    .unwrap_or_else(|error| {
                        let reason = error.map_or_else(
                            || format!("the document holds no blob {}", underlay.lines_blob()),
                            |error| error.to_string(),
                        );
                        self.digest = self.catalog.format(
                            "error-underlay-lines",
                            &BTreeMap::from([("name", underlay.name.clone()), ("reason", reason)]),
                        );
                        Arc::new(UnderlayLines::default())
                    });
                let geometry = Arc::new(UnderlayGeometry::new(key, &lines));
                self.render.underlays.insert(key, Arc::clone(&geometry));
                self.render
                    .underlay_extents
                    .get_mut()
                    .insert(key, underlay_extent(&lines));
                self.build_underlay_snap_index(key, lines);
                geometry
            };
            let visible = if shown(underlay.tag) {
                underlay
                    .layers
                    .iter()
                    .take(geometry.layer_count())
                    .enumerate()
                    .filter(|(_, layer)| shown(layer.tag))
                    .map(|(index, _)| index)
                    .collect()
            } else {
                Vec::new()
            };
            let placement = match dragged {
                Some((id, edit)) if id == underlay.id => edit.compose(underlay.world_transform()),
                _ => underlay.world_transform(),
            };
            draws.push(UnderlayDraw {
                geometry,
                visible,
                placement,
            });
        }
        let named = |key: &[u8; 32]| {
            snapshot
                .underlays()
                .any(|underlay| underlay.lines_key() == *key)
        };
        self.render.underlays.retain(|key, _| named(key));
        self.render
            .underlay_snaps
            .get_mut()
            .retain(|key, _| named(key));
        self.render
            .underlay_extents
            .get_mut()
            .retain(|key, _| named(key));
        self.render
            .underlay_lines
            .get_mut()
            .retain(|key, _| named(key));
        self.render
            .underlay_object_trees
            .get_mut()
            .retain(|key, _| named(key));
        self.render
            .underlay_decoded
            .get_mut()
            .retain(|(blob, layers), _| {
                snapshot.underlays().any(|underlay| {
                    underlay.lines_sha256 == *blob && underlay.layers.len() == *layers
                })
            });
        draws
    }

    /// The world box of what every shown underlay draws on its shown layers, where it is
    /// placed: one `(origin, size)` per underlay, as [`Self::active_frame_bounds`] takes it.
    pub(crate) fn shown_underlay_bounds(&self, snapshot: &Snapshot) -> Vec<(Vec3, Vec3)> {
        let shown = |tag| snapshot.tag(tag).is_some_and(|tag| tag.visible());
        snapshot
            .underlays()
            .filter(|underlay| shown(underlay.tag))
            .filter_map(|underlay| {
                let extent = self.cached_underlay_extent(underlay);
                let corners = underlay
                    .layers
                    .iter()
                    .zip(extent.iter())
                    .filter(|(layer, _)| shown(layer.tag))
                    .filter_map(|(_, layer)| layer.bounds_mm)
                    .flat_map(|[low, high]| {
                        [[low[0], low[1]], [high[0], low[1]], [low[0], high[1]], high]
                    })
                    .map(|[x, y]| {
                        transform_model_point(underlay.world_transform(), Vec3::new(x, y, 0.0))
                    });
                let [minimum, maximum] = bounds_of(corners)?;
                Some((minimum, maximum - minimum))
            })
            .collect()
    }

    /// The straight lines and texts of every underlay a tag shows, by tag: an underlay's
    /// own tag counts all its layers, a layer's tag that layer. A tag without underlay
    /// content is absent.
    pub(crate) fn underlay_counts_by_tag(
        &self,
        snapshot: &Snapshot,
    ) -> BTreeMap<TagId, [usize; 2]> {
        let mut counts = BTreeMap::<TagId, [usize; 2]>::new();
        for underlay in snapshot.underlays() {
            let extent = self.cached_underlay_extent(underlay);
            for (layer, drawn) in underlay.layers.iter().zip(extent.iter()) {
                for tag in [underlay.tag, layer.tag] {
                    let count = counts.entry(tag).or_default();
                    count[0] += drawn.lines;
                    count[1] += drawn.texts;
                }
            }
        }
        counts.retain(|_, count| *count != [0, 0]);
        counts
    }

    /// The lines and objects of `underlay`, decoded from the document's blob the first
    /// time; `None` while the document holds no readable blob of them.
    pub(crate) fn cached_underlay_lines(&self, underlay: &Underlay) -> Option<Arc<UnderlayLines>> {
        self.underlay_lines_or_reason(underlay).ok()
    }

    /// The children and bounds of the objects of `lines`, the
    /// [`Self::cached_underlay_lines`] of `underlay`, worked out the first time.
    pub(crate) fn cached_object_tree(
        &self,
        underlay: &Underlay,
        lines: &UnderlayLines,
    ) -> Arc<ObjectTree> {
        let key = underlay.lines_key();
        if let Some(tree) = self.render.underlay_object_trees.borrow().get(&key) {
            return Arc::clone(tree);
        }
        #[cfg(test)]
        OBJECT_TREES.with(|built| built.set(built.get() + 1));
        let tree = Arc::new(ObjectTree::new(lines));
        self.render
            .underlay_object_trees
            .borrow_mut()
            .insert(key, Arc::clone(&tree));
        tree
    }

    /// The lines and objects of `underlay` with its edits, or why they cannot be read:
    /// `None` when the document holds no blob of them. Its blob is decoded once and kept while an underlay names it, so an
    /// edit applies the edits to the decoded blob instead of decoding it again (review
    /// 2026-10-29 UI-62).
    fn underlay_lines_or_reason(
        &self,
        underlay: &Underlay,
    ) -> Result<Arc<UnderlayLines>, Option<UnderlayLinesError>> {
        let key = underlay.lines_key();
        if let Some(lines) = self.render.underlay_lines.borrow().get(&key) {
            return Ok(Arc::clone(lines));
        }
        let blob = underlay.lines_blob();
        let decoded_key = (underlay.lines_sha256, underlay.layers.len());
        let cached = self
            .render
            .underlay_decoded
            .borrow()
            .get(&decoded_key)
            .cloned();
        let decoded = match cached {
            Some(decoded) => decoded,
            None => {
                let bytes = self.file.container_data.blobs();
                let bytes = bytes.get(&blob).ok_or(None)?;
                #[cfg(test)]
                BLOB_DECODES.with(|decodes| decodes.set(decodes.get() + 1));
                let decoded =
                    Arc::new(UnderlayLines::decode(bytes, underlay.layers.len()).map_err(Some)?);
                self.render
                    .underlay_decoded
                    .borrow_mut()
                    .insert(decoded_key, Arc::clone(&decoded));
                decoded
            }
        };
        let lines = if underlay.edits.is_empty() {
            decoded
        } else {
            Arc::new(
                underlay
                    .with_edits(UnderlayLines::clone(&decoded))
                    .map_err(Some)?,
            )
        };
        self.render
            .underlay_lines
            .borrow_mut()
            .insert(key, Arc::clone(&lines));
        Ok(lines)
    }

    /// The layer extents of `underlay`'s lines, decoded from the document's blob the first
    /// time, or taken from lines the bridge already decoded; a blob the document lacks or
    /// cannot decode has no layers, kept as [`Self::underlay_draws`] keeps it, so it is not
    /// decoded again each frame (review 2026-10-24 UI-27).
    fn cached_underlay_extent(&self, underlay: &Underlay) -> UnderlayExtent {
        let key = underlay.lines_key();
        if let Some(extent) = self.render.underlay_extents.borrow().get(&key) {
            return Arc::clone(extent);
        }
        let extent = self
            .cached_underlay_lines(underlay)
            .map_or_else(|| Arc::from([]), |lines| underlay_extent(&lines));
        self.render
            .underlay_extents
            .borrow_mut()
            .insert(key, Arc::clone(&extent));
        extent
    }
}
