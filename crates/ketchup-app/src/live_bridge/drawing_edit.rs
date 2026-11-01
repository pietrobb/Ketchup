//! The `drawing` tool: 2D CAD on an imported drawing. `remove` takes top-level objects
//! (ids of `inspect action=underlay_objects`, or all that meet a box) out of it, `draw`
//! adds lines, polylines, arcs, circles and texts on a layer, and `move` moves, turns,
//! mirrors or scales objects or adds such copies of them; each is one Undo step, and
//! everything that draws, snaps to or asks about the underlay then sees the edited drawing.

use super::*;
use crate::app::underlay_edit::{FindRegions, UnderlayEditError};
use crate::app::underlay_export::{ClientUnderlayExport, UnderlayExportError};
use crate::app::underlay_extrude::{AreaPullError, AreaRefusal};
use ketchup_model::underlay::drawn::{in_lines, inverse, similarity};
use ketchup_model::underlay::edits::UnderlayObjectError;
use ketchup_model::underlay::line_edit::LineEdit;
use ketchup_model::underlay::object_query::{ObjectFilter, ObjectIndex};
use ketchup_model::underlay::objects::apply;

impl LiveBridge {
    /// The `drawing_*` methods: each changes an underlay's drawing or builds from it.
    pub(super) fn drawing_request(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        match request {
            Request::DrawingRemove { .. } => self.drawing_remove(app, request, ui_busy, cancelled),
            Request::DrawingDraw { .. } => self.drawing_draw(app, request, ui_busy, cancelled),
            Request::DrawingMove { .. } => self.drawing_move(app, request, ui_busy, cancelled),
            Request::DrawingExport { .. } => {
                Self::drawing_export_target(app, request, ui_busy, cancelled).and_then(|target| {
                    target
                        .write_here(app)
                        .map(|written| target.written(written))
                        .map_err(|error| target.refuse(error))
                })
            }
            Request::DrawingExtrude { .. } => {
                self.drawing_extrude(app, request, ui_busy, cancelled)
            }
            Request::DrawingView { .. } => self.drawing_view(app, request, ui_busy, cancelled),
            Request::DrawingSolidFromViews { .. } => {
                self.drawing_solid_from_views(app, request, ui_busy, cancelled)
            }
            _ => self.drawing_line_edit(app, request, ui_busy, cancelled),
        }
    }

    /// `drawing_remove`: takes `object_ids` out of the underlay's drawing.
    pub(super) fn drawing_remove(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::DrawingRemove {
            expected,
            underlay_id,
            object_ids,
            within_mm,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?;
        let id = underlay.id;
        let object_ids = match (object_ids, within_mm) {
            (Some(ids), None) => ids.clone(),
            (None, Some(area)) if area.iter().flatten().all(|value| value.is_finite()) => {
                let lines = app
                    .cached_underlay_lines(underlay)
                    .ok_or("entity_not_found")?;
                let place = super::underlay_objects::Placed { underlay };
                ObjectIndex::new(&lines).find(&ObjectFilter {
                    within_mm: Some(place.local_area(*area)),
                    top_level: true,
                    ..ObjectFilter::default()
                })
            }
            _ => return Err("invalid_params"),
        };
        if object_ids.is_empty() && within_mm.is_some() {
            return Err("entity_not_found");
        }
        Self::require_request_authority(cancelled)?;
        let changes = app
            .remove_underlay_objects(id, &object_ids, FindRegions::Here)
            .map_err(edit_failure)?;
        self.query.invalidate();
        let snapshot = app.document.current();
        let left = snapshot
            .underlay(id)
            .and_then(|underlay| app.cached_underlay_lines(underlay))
            .map(|lines| ObjectIndex::new(&lines).present());
        let removed = object_ids.iter().collect::<BTreeSet<_>>();
        Ok(with_followed(
            json!({
                "removed": removed.len(),
                "object_ids": removed,
                "underlay_id": id.0,
                "objects_left": left,
                "dirty": app.is_dirty(),
            }),
            &changes,
        ))
    }

    /// `drawing_draw`: draws shapes given in document millimetres into a layer.
    pub(super) fn drawing_draw(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::DrawingDraw {
            expected,
            underlay_id,
            layer,
            shapes,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?;
        let id = underlay.id;
        let to_lines = inverse(&underlay.plane_map()).ok_or("invalid_params")?;
        let shapes = shapes
            .iter()
            .map(|shape| shape.transformed(&to_lines))
            .collect::<Vec<_>>();
        let first = app
            .cached_underlay_lines(underlay)
            .ok_or("entity_not_found")?
            .objects()
            .objects
            .len();
        let count = shapes.len();
        Self::require_request_authority(cancelled)?;
        let changes = app
            .draw_into_underlay(id, layer, shapes, FindRegions::Here)
            .map_err(edit_failure)?;
        self.query.invalidate();
        Ok(with_followed(
            json!({
                "drawn": count,
                "object_ids": (first..first + count).collect::<Vec<_>>(),
                "underlay_id": id.0,
                "layer": layer,
                "dirty": app.is_dirty(),
            }),
            &changes,
        ))
    }

    /// `drawing_move`: moves top-level objects, or adds moved copies of them.
    pub(super) fn drawing_move(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::DrawingMove {
            expected,
            underlay_id,
            object_ids,
            by_mm,
            rotate_deg,
            scale,
            about_mm,
            mirror_line_mm,
            copy,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        if by_mm.is_none() && rotate_deg.is_none() && scale.is_none() && mirror_line_mm.is_none() {
            return Err("invalid_params");
        }
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?;
        let id = underlay.id;
        let map = similarity(
            by_mm.unwrap_or([0.0; 2]),
            rotate_deg.unwrap_or(0.0),
            scale.unwrap_or(1.0),
            about_mm.unwrap_or([0.0; 2]),
            *mirror_line_mm,
        );
        let map = in_lines(&map, &underlay.plane_map()).ok_or("invalid_params")?;
        let first = app
            .cached_underlay_lines(underlay)
            .ok_or("entity_not_found")?
            .objects()
            .objects
            .len();
        Self::require_request_authority(cancelled)?;
        let changes = app
            .transform_underlay_objects(id, object_ids, &map, *copy, FindRegions::Here)
            .map_err(edit_failure)?;
        self.query.invalidate();
        let objects = object_ids.iter().collect::<BTreeSet<_>>();
        let snapshot = app.document.current();
        let after = snapshot
            .underlay(id)
            .and_then(|underlay| app.cached_underlay_lines(underlay))
            .map_or(first, |lines| lines.objects().objects.len());
        let mut answer = json!({
            "copy": copy,
            "object_ids": objects,
            "underlay_id": id.0,
            "dirty": app.is_dirty(),
        });
        if *copy {
            // A copy of an insert or dimension adds what it places too; the top-level ones
            // are the new objects placed by none.
            let lines = snapshot
                .underlay(id)
                .and_then(|underlay| app.cached_underlay_lines(underlay));
            let new = lines.map_or_else(Vec::new, |lines| {
                (first..after)
                    .filter(|object| lines.objects().objects[*object].parent.is_none())
                    .collect()
            });
            answer["new_object_ids"] = json!(new);
        }
        Ok(with_followed(answer, &changes))
    }

    /// `drawing_trim`, `drawing_extend` and `drawing_offset`: one straight line of the
    /// drawing trimmed back, extended or offset; it keeps its id, and a second piece of it
    /// and an offset are new objects.
    pub(super) fn drawing_line_edit(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let (expected, underlay_id, object_id, at_mm, distance_mm, action) = match request {
            Request::DrawingTrim {
                expected,
                underlay_id,
                object_id,
                at_mm,
            } => (expected, underlay_id, *object_id, *at_mm, None, "trim"),
            Request::DrawingExtend {
                expected,
                underlay_id,
                object_id,
                at_mm,
            } => (expected, underlay_id, *object_id, *at_mm, None, "extend"),
            Request::DrawingOffset {
                expected,
                underlay_id,
                object_id,
                at_mm,
                distance_mm,
            } => (
                expected,
                underlay_id,
                *object_id,
                *at_mm,
                *distance_mm,
                "offset",
            ),
            _ => return Err("invalid_request"),
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?;
        let id = underlay.id;
        let to_lines = inverse(&underlay.plane_map()).ok_or("invalid_params")?;
        let at = apply(&to_lines, at_mm);
        let scale = underlay.scale();
        let edit = match action {
            "trim" => LineEdit::Trim { pick_mm: at },
            "extend" => LineEdit::Extend { pick_mm: at },
            _ => LineEdit::Offset {
                distance_mm: distance_mm.map(|distance| distance / scale),
                side_mm: at,
            },
        };
        let first = app
            .cached_underlay_lines(underlay)
            .ok_or("entity_not_found")?
            .objects()
            .objects
            .len();
        Self::require_request_authority(cancelled)?;
        let changes = app
            .edit_underlay_line(id, object_id, edit, FindRegions::Here)
            .map_err(edit_failure)?;
        self.query.invalidate();
        let after = app
            .document
            .current()
            .underlay(id)
            .and_then(|underlay| app.cached_underlay_lines(underlay))
            .map_or(first, |lines| lines.objects().objects.len());
        Ok(with_followed(
            json!({
                "action": action,
                "object_id": object_id,
                "new_object_ids": (first..after).collect::<Vec<_>>(),
                "underlay_id": id.0,
                "dirty": app.is_dirty(),
            }),
            &changes,
        ))
    }

    /// `drawing_export`: what writing the drawing with its changes to the DXF or DWG file
    /// `path` takes, checked before the worker writes it.
    pub(super) fn drawing_export_target(
        app: &KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<DrawingExportTarget, &'static str> {
        let Request::DrawingExport {
            expected,
            underlay_id,
            path,
            source,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let drawing = Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("dxf") || extension.eq_ignore_ascii_case("dwg")
            });
        if !drawing || !super::drawings::local_absolute_path(path) {
            return Err(failure(
                "invalid_path",
                "path is a local absolute path ending in .dxf or .dwg.",
                json!({"path": path}),
            ));
        }
        // No one confirms replacing a file a client names (SEC-60, as MCP-3).
        super::drawings::refuse_existing(path)?;
        if let Some(source) = source
            && !super::drawings::local_absolute_path(source)
        {
            return Err(failure(
                "invalid_path",
                "source is the local absolute path of the DXF or DWG file the drawing was \
                 imported from.",
                json!({"source": source}),
            ));
        }
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?;
        let edits = &underlay.edits;
        let changed = json!({
            "removed": edits.removed_objects.len(),
            "moved": edits.moved_objects.len(),
            "stretched": edits.stretched_objects.len(),
            "added": edits.added.len(),
        });
        Self::require_request_authority(cancelled)?;
        Ok(DrawingExportTarget {
            id: underlay.id,
            source: source.clone(),
            path: path.clone(),
            changed,
        })
    }
}

/// A drawing a client asked to write back, checked on the window's thread.
pub(super) struct DrawingExportTarget {
    id: ketchup_model::underlay::UnderlayId,
    source: Option<String>,
    path: String,
    changed: Value,
}

impl DrawingExportTarget {
    /// Starts the worker on a thread.
    pub(super) fn start(
        &self,
        app: &KetchupApp,
        context: &egui::Context,
    ) -> Result<ClientUnderlayExport, UnderlayExportError> {
        app.start_client_underlay_export(
            self.id,
            self.source.as_deref().map(Path::new),
            Path::new(&self.path),
            context,
        )
    }

    /// The in-process harness's drawing export: the job a client's request starts,
    /// waited for here.
    pub(super) fn write_here(
        &self,
        app: &KetchupApp,
    ) -> Result<ketchup_application::program_isolation::UnderlayWritten, UnderlayExportError> {
        let written = self
            .start(app, &egui::Context::default())
            .and_then(|export| {
                export.receiver.recv().unwrap_or_else(|_| {
                    Err(UnderlayExportError::Io(std::io::Error::other(
                        "the thread that writes the drawing stopped",
                    )))
                })
            });
        ClientUnderlayExport::publish(written)
    }

    /// Why the drawing was not written, recorded with its cause for the client.
    pub(super) fn refuse(&self, error: UnderlayExportError) -> &'static str {
        let code = drawing_export_code(&error);
        record_rejection(
            &rejected(code).caused_by(error),
            json!({"path": self.path, "underlay_id": self.id.0}),
        );
        code
    }

    /// The client's answer for the drawing written.
    pub(super) fn written(
        &self,
        written: ketchup_application::program_isolation::UnderlayWritten,
    ) -> Value {
        json!({
            "path": self.path,
            "bytes": written.bytes,
            "underlay_id": self.id.0,
            "changes": self.changed,
            // The records of a DWG source its read left out are not in the file either.
            "skipped_records": written.skipped.iter().map(|(record_type, count)| json!({
                "record_type": record_type,
                "count": count,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The host code of why a drawing was not written back, whose hint repairs that cause
/// (review 2026-10-29 CD-72).
pub(super) fn drawing_export_code(error: &UnderlayExportError) -> &'static str {
    use crate::export_bundle::ExportTransactionError;
    use ketchup_application::program_isolation::{
        UnderlayReadError, UnderlayRefusal, UnderlayWorkerError,
    };
    use ketchup_model::import::DxfEditRefusal;
    match error {
        UnderlayExportError::NotFound(..) => "invalid_params",
        UnderlayExportError::NoSource(..) | UnderlayExportError::NoLines(..) => {
            "drawing_source_unavailable"
        }
        UnderlayExportError::Worker(UnderlayWorkerError::Read(UnderlayReadError::Refused(
            refusal,
        ))) => match refusal {
            UnderlayRefusal::DwgWrite(..) => "dwg_conversion_failed",
            UnderlayRefusal::NotWritten(
                DxfEditRefusal::NoHandle { .. }
                | DxfEditRefusal::HandleMissing { .. }
                | DxfEditRefusal::NotMappable { .. },
            ) => "drawing_edit_not_writable",
            UnderlayRefusal::NotWritten(..)
            | UnderlayRefusal::Dwg(..)
            | UnderlayRefusal::UnitsRequired
            | UnderlayRefusal::Dxf { .. }
            | UnderlayRefusal::File { .. } => "drawing_source_unavailable",
        },
        UnderlayExportError::Worker(UnderlayWorkerError::Read(UnderlayReadError::Cancelled)) => {
            "request_cancelled"
        }
        UnderlayExportError::Worker(..) => "drawing_worker_failed",
        UnderlayExportError::SameAsSource(..)
        | UnderlayExportError::Publish(
            ExportTransactionError::TargetChanged { .. }
            | ExportTransactionError::OriginalPreserved { .. },
        ) => "drawings_target_exists",
        UnderlayExportError::Io(..) | UnderlayExportError::Publish(..) => "drawings_write_failed",
    }
}

/// The rejection of a removal, with the object to remove instead of one an insert or
/// dimension placed. The whole error is kept as the rejection's cause.
impl LiveBridge {
    /// `drawing_extrude`: a closed area of the drawing pulled up into a solid.
    pub(super) fn drawing_extrude(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::DrawingExtrude {
            expected,
            underlay_id,
            at_mm,
            object_id,
            height_mm,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        if at_mm.is_some() == object_id.is_some() {
            return Err(super::program_access::invalid(
                "at_mm",
                "Give exactly one of at_mm or object_id.",
                "at_mm for the closed region around a point, object_id for one closed polyline or hatch.",
            ));
        }
        let snapshot = app.document.current();
        let id = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?.id;
        // A refusal answers with its own cause (review 2026-10-29 UO-63).
        let area = app
            .underlay_area(id, *at_mm, *object_id)
            .map_err(refused_area)?
            .ok_or_else(|| {
                super::program_access::invalid(
                    "at_mm",
                    "No closed area of the shown layers is there.",
                    "Ask inspect action=underlay_regions with point_mm, or underlay_objects for a closed polyline or hatch.",
                )
            })?;
        Self::require_request_authority(cancelled)?;
        let pulled = app
            .pull_underlay_area(&area, *height_mm)
            .map_err(|error| match error {
                AreaPullError::Height => super::program_access::invalid(
                    "height_mm",
                    "The height must be a positive length.",
                    "Give the height in millimetres, e.g. 2600.",
                ),
                AreaPullError::NotWritable(refusal) => refused_area(refusal),
                error => {
                    record_rejection(&rejected("commit_rejected").caused_by(error), json!({}));
                    "commit_rejected"
                }
            })?;
        self.query.invalidate();
        let mut answer = json!({
            "name": pulled.name,
            "bound_to_drawing": pulled.bound,
            "height_mm": height_mm,
            "underlay_id": id.0,
            "undo_steps": app.undo_step_count(),
            "dirty": app.is_dirty(),
        });
        // Review 2026-10-31 UO-83: holes of the room drawn on other layers than its walls.
        if let Some(left_out) = pulled.left_out {
            answer["holes_left_out"] = json!({
                "count": left_out.count,
                "layers": left_out.layers,
                "hint": "Holes drawn on other layers than the walls are not cut; pull the area with program action=from_underlay naming those layers to cut them.",
            });
        }
        Ok(answer)
    }
}

/// `answer` with the program sections whose outline the edit changed, as
/// `areas_changed` (review 2026-10-29 UO-64), when it changed any.
fn with_followed(mut answer: Value, changes: &[super::underlay_binding::FollowedChange]) -> Value {
    if !changes.is_empty() {
        answer["areas_changed"] = changes
            .iter()
            .map(|change| json!({"name": change.name, "was": change.was, "now": change.now}))
            .collect();
    }
    answer
}

/// The code of `refusal`, its rejection recorded again for the answer.
fn refused_area(refusal: AreaRefusal) -> &'static str {
    super::restore_error_details(refusal.recorded);
    refusal.code
}

fn edit_failure(error: UnderlayEditError) -> &'static str {
    let code = match &error {
        UnderlayEditError::Commit(..) | UnderlayEditError::FindingRegions(..) => "commit_rejected",
        UnderlayEditError::Plan(UnderlayObjectError::NotFound(..)) => "entity_not_found",
        UnderlayEditError::NoLines | UnderlayEditError::Plan(..) => "invalid_params",
    };
    let details = match &error {
        UnderlayEditError::Plan(UnderlayObjectError::Placed { object, parent }) => {
            json!({"object_id": object, "remove_instead": parent})
        }
        UnderlayEditError::Plan(
            UnderlayObjectError::NotFound(object)
            | UnderlayObjectError::NotALine(object)
            | UnderlayObjectError::NoCrossing(object),
        ) => {
            json!({"object_id": object})
        }
        UnderlayEditError::Plan(UnderlayObjectError::UnknownLayer(layer)) => {
            json!({"layer": layer})
        }
        UnderlayEditError::Plan(UnderlayObjectError::InvalidShape(index)) => {
            json!({"shape_index": index})
        }
        UnderlayEditError::Plan(UnderlayObjectError::TooLarge(size)) => json!({
            "objects": size.objects,
            "max_objects": ketchup_model::tolerance::limits::DRAWING_ENTITIES,
            "bytes": size.bytes,
            "max_bytes": ketchup_model::tolerance::limits::UNDERLAY_LINES_BYTES,
        }),
        _ => json!({}),
    };
    record_rejection(&rejected(code).caused_by(error), details);
    code
}
