//! `file action=export_drawings`: the project drawings of the visible layers as
//! a numbered set of sheets (a plan per storey, sections, elevations), one PDF
//! or a DXF or DWG per sheet, with the title block and format kept in the
//! document; with `sheet_id`, a drawing sheet of the document as DXF or DWG.

use super::project_drawings_job::exports_project_drawings;
use super::{LiveBridge, Request, Stamp, failed_because, failure};
use crate::KetchupApp;
use crate::app::drawing_sheet_export::{
    DrawingSheetExportError, ExportedDrawingSheet, SheetFileFormat, drawing_sheet_bytes,
    drawing_sheet_target, publish_drawing_sheet,
};
use crate::app::project_drawings::{
    ExportedSet, ProjectDrawingsError, ProjectDrawingsJob, SetFileFormat, SetOwner, SetTargets,
    SheetRequest, project_drawings_targets, publish_project_drawings, written_files,
};
use crate::app::underlay_export::PublishGate;
use crate::export_bundle::ExportTransactionError;
use ketchup_manufacturing::dxf_annotations::MAX_PLAN_SCALE;
use ketchup_manufacturing::project_drawings::DrawingSet;
use ketchup_manufacturing::title_block::{
    SheetFormat, SheetLanguage, SheetSettings, StoreyLevels, TitleField,
};
use ketchup_model::drawing::DrawingSheetId;
use ketchup_model::drawing_export::{DxfCurves, SheetAnnotations};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// The parameters of `file action=export_drawings`: the project set of the
/// visible layers, its settings kept in the document first, or with
/// `sheet_id` a drawing sheet of the document.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportDrawingsRequest {
    #[serde(default)]
    pub expected: Option<Stamp>,
    pub path: String,
    #[serde(default)]
    pub sheet_id: Option<u64>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub title_block: BTreeMap<TitleField, String>,
    /// Only the plan, with the annotations, rather than all seven views.
    #[serde(default)]
    pub plan_only: bool,
    /// N of the scale 1:N the sheet is drawn at; the sheet picks it when absent.
    #[serde(default)]
    pub drawing_scale: Option<f64>,
    /// The language the sheets are titled in, kept in the document:
    /// cs, sk or en, auto for the window's; absent keeps the document's.
    #[serde(default)]
    pub sheet_language: Option<String>,
    /// Replaces a PDF or a drawing sheet's file at `path` that is not the
    /// one the document last wrote there (review 2026-11-04 MCP-140), as
    /// the user agreed when asked (r19 SEC-161).
    #[serde(default)]
    pub replace: bool,
    /// Takes the DXF or DWG set at `path` of another project for this
    /// one's, as the user agreed when asked (review 2026-11-04 r19 MF-162).
    #[serde(default)]
    pub take_over: bool,
    /// The height of ±0,000 in whole millimetres, kept in the document:
    /// `"auto"` finds it from the model; absent keeps the document's
    /// (review 2026-11-04 r19 P3).
    #[serde(default)]
    pub zero_level_mm: Option<Value>,
    /// The table of storeys, kept in the document: a list of
    /// `{finished_floor_mm, structural_mm}`, or `"auto"` to find them from
    /// the model; absent keeps the document's (review 2026-11-08 MF-241).
    #[serde(default)]
    pub storeys: Option<Value>,
    /// The altitude of ±0,000 above sea level in whole millimetres, kept
    /// in the document and written beside its level mark; `"none"`
    /// writes none; absent keeps the document's.
    #[serde(default)]
    pub zero_altitude_mm: Option<Value>,
    /// The direction of north in degrees counter-clockwise from the model's
    /// +y, kept in the document; `"none"` takes it away; absent keeps the
    /// document's (review 2026-11-08 §5.2).
    #[serde(default)]
    pub north_deg: Option<Value>,
}

fn sheet_format(name: &str) -> Result<Option<SheetFormat>, &'static str> {
    if name.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    SheetFormat::ALL
        .into_iter()
        .find(|format| format.name().eq_ignore_ascii_case(name))
        .map(Some)
        .ok_or("invalid_sheet_format")
}

impl LiveBridge {
    pub(super) fn export_drawings(
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
    ) -> Result<Value, &'static str> {
        if exports_project_drawings(&request) {
            // The project set, drawn here: only the in-process caller waits
            // for it, as a client's export is a job (review 2026-11-01 UI-100).
            let (started, job) = start_export(app, request, ui_busy)?;
            let made = job.render(started.file_format, &AtomicBool::new(false));
            return finish_export(app, started, made).map_err(export_failed);
        }
        let export = Self::sheet_export_of(app, request, ui_busy)?;
        let target = export.target.clone();
        let made = export.run(&PublishGate::default());
        finish_sheet(app, &target, made).map_err(|refusal| sheet_refused(&target, refusal))
    }

    /// The export of the drawing sheet `request` names, checked; a client's
    /// runs it as a job (review 2026-11-06 P3).
    pub(super) fn sheet_export_of(
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
    ) -> Result<SheetExport, &'static str> {
        let Request::ExportDrawings(ExportDrawingsRequest {
            expected,
            path,
            sheet_id: Some(id),
            format,
            title_block,
            plan_only,
            drawing_scale,
            sheet_language,
            replace,
            take_over,
            zero_level_mm,
            storeys,
            zero_altitude_mm,
            north_deg,
        }) = request
        else {
            return Err(failure(
                "invalid_params",
                "export_drawings of a drawing sheet needs its sheet_id.",
                json!({}),
            ));
        };
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        if format.is_some()
            || !title_block.is_empty()
            || plan_only
            || drawing_scale.is_some()
            || sheet_language.is_some()
            || take_over
            || zero_level_mm.is_some()
            || storeys.is_some()
            || zero_altitude_mm.is_some()
            || north_deg.is_some()
        {
            return Err(failure(
                "invalid_params",
                "format, title_block, plan_only, drawing_scale, sheet_language, zero_level_mm, storeys, zero_altitude_mm, north_deg and take_over belong to the project sheets; a drawing sheet keeps its own page, views, scale and title block.",
                json!({"sheet_id": id}),
            ));
        }
        let format =
            SheetFileFormat::of_path(Path::new(&path)).filter(|_| local_absolute_path(&path));
        let Some(format) = format else {
            return Err(failure(
                "invalid_path",
                "A drawing sheet is written to a local absolute path ending in .dxf or .dwg (no network share, device or stream).",
                json!({"path": path}),
            ));
        };
        let target = SheetTarget {
            path: PathBuf::from(path),
            id: DrawingSheetId(id),
            format,
        };
        if app.exact.task.is_some() {
            return Err(sheet_failed(&target, DrawingSheetExportError::Evaluating));
        }
        // Whether the file there is the document's own is told off the
        // window, by its SHA-256; one there at all is asked about.
        if replace
            && std::fs::symlink_metadata(&target.path).is_ok()
            && let Some(code) = replace_declined(app, &target.path)
        {
            return Err(code);
        }
        Ok(SheetExport {
            target,
            replace,
            snapshot: app.document.current(),
            results: app.exact.results.clone(),
            words: app.parts_list_words(),
            written: app.written_drawings(),
        })
    }
}

/// N of a scale 1:N a sheet is drawn at: a whole number from 1 to 100 000.
fn sheet_scale(scale: f64) -> Result<u32, &'static str> {
    let whole = (scale.fract() == 0.0 && (1.0..=MAX_PLAN_SCALE).contains(&scale))
        .then(|| format!("{scale:.0}").parse::<u32>().ok())
        .flatten();
    whole.ok_or_else(|| {
        failure(
            "invalid_params",
            format!(
                "drawing_scale is N of the scale 1:N of the sheet, a whole number from 1 to {MAX_PLAN_SCALE}."
            ),
            json!({"drawing_scale": scale}),
        )
    })
}

/// Never replaces a file: no one confirms an overwrite on this path; the
/// window's own export asks before replacing.
pub(super) fn refuse_existing(path: &str) -> Result<(), &'static str> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(failure(
            "drawings_target_exists",
            "A file already exists at this path; export the drawings to a new file name.",
            json!({"path": path}),
        ));
    }
    Ok(())
}

/// Whether the file at `path`, of SHA-256 `existing`, is kept: it is not the
/// last one the document wrote there and the client did not say `replace`. No
/// one confirms an overwrite for a client, as the window's Save dialog does for
/// the user, so a PDF the user added pages to or a DWG of someone else's is
/// kept (review 2026-11-04 MCP-140) and the client is told to ask the user
/// (r19 SEC-161).
fn kept_unless_asked(app: &KetchupApp, path: &Path, existing: Option<&str>, replace: bool) -> bool {
    existing.is_some_and(|sha256| !replace && !app.wrote_drawing(path, sha256))
}

/// The answer to an export [`kept_unless_asked`] refused.
fn not_its_own(path: &Path) -> &'static str {
    failure(
        "drawings_target_exists",
        format!(
            "{} is not the file this document's last export wrote there (another program's, or changed since), so it is not replaced. Export to another path, or export again with replace: true and the window asks the user whether to replace it.",
            path.display()
        ),
        json!({"path": path.display().to_string()}),
    )
}

/// The code refusing a client's `replace` or `take_over` of the file at
/// `path`, which the export did not write, unless the user agrees in the
/// window (review 2026-11-08 SEC-182). Before, the schema told the AI to ask
/// and the flag alone replaced the file.
fn replace_declined(app: &mut KetchupApp, path: &Path) -> Option<&'static str> {
    app.authorize_overwrite(path, path.to_string_lossy().as_bytes())
        .map_err(|rejection| {
            failure(
                "drawings_replace_declined",
                format!(
                    "{} {} was not replaced and nothing was written; export to another path, or ask the user again.",
                    rejection.reason_text(),
                    path.display()
                ),
                json!({"path": path.display().to_string()}),
            )
        })
        .err()
}

/// The export transaction's own reason within `error`.
fn transaction_error<'a>(
    error: &'a (dyn std::error::Error + 'static),
) -> Option<&'a ExportTransactionError> {
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(found) = error.downcast_ref::<ExportTransactionError>() {
            return Some(found);
        }
        // An I/O error made of another error does not give it as its source.
        next = match error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
        {
            Some(inner) => Some(inner as &(dyn std::error::Error + 'static)),
            None => error.source(),
        };
    }
    None
}

/// The answer to a drawing that could not be written because of `error`; a
/// file another program holds open (a PDF viewer, a CAD program, which
/// Windows will not let be replaced) or one the export may not replace is
/// named, with what the user can do about it.
fn write_failed(error: impl std::error::Error + Send + Sync + 'static) -> &'static str {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    let (code, reason, file) = match transaction_error(&error) {
        Some(ExportTransactionError::Io { path, cause })
            if cfg!(windows)
                && matches!(
                    cause.raw_os_error(),
                    Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
                ) =>
        {
            (
                "drawings_target_in_use",
                "is open in another program (a PDF viewer or a CAD program), so it cannot be replaced; close it there and export again",
                path,
            )
        }
        // The file there is another program's; the one it replaced is kept
        // beside it, hidden, and named (review 2026-11-04 P3).
        Some(ExportTransactionError::OriginalPreserved { path, preserved }) => {
            return failure(
                "drawings_target_exists",
                format!(
                    "{} was replaced by another program while the drawings were written, so they were not written there; the file it replaced is kept as {}. Rename it back if it is the one you want, or export to another path.",
                    path.display(),
                    preserved.display()
                ),
                json!({
                    "path": path.display().to_string(),
                    "preserved": preserved.display().to_string(),
                }),
            );
        }
        Some(ExportTransactionError::TargetChanged { path }) => (
            "drawings_target_exists",
            "is not a file this project's earlier export of these drawings wrote (or it changed while they were drawn), so it is not replaced; rename or move it, or export the drawings to another folder",
            path,
        ),
        _ => return failed_because("drawings_write_failed", error),
    };
    failure(
        code,
        format!(
            "{} {reason}. Nothing was written; the file is as it was.",
            file.display()
        ),
        json!({"path": file.display().to_string()}),
    )
}

/// Which sheet is written where, as a format.
#[derive(Clone)]
pub(super) struct SheetTarget {
    path: PathBuf,
    id: DrawingSheetId,
    format: SheetFileFormat,
}

/// A drawing sheet's export as it runs off the window (review 2026-11-06 P3:
/// the SHA-256 of the file at the path was taken on the window's thread):
/// what the sheet is drawn from and which files are the document's own.
pub(super) struct SheetExport {
    pub(super) target: SheetTarget,
    replace: bool,
    snapshot: ketchup_model::document::Snapshot,
    results: ketchup_model::exact_product::ExactResultRegistry,
    words: ketchup_model::drawing::PartsListWords,
    /// The drawing files the document wrote, by their key.
    written: std::collections::BTreeMap<String, String>,
}

impl SheetTarget {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

/// Why a sheet was not written.
pub(super) enum SheetRefusal {
    /// The file at the path is not the one the document last wrote there.
    NotItsOwn,
    /// Cancelled before the sheet took the place of the file.
    Cancelled,
    Failed(DrawingSheetExportError),
}

pub(super) type SheetMade = Result<ExportedDrawingSheet, SheetRefusal>;

impl SheetExport {
    /// Draws drawing sheet as DXF, or that DXF as an AutoCAD 2018 DWG (exact
    /// views, hidden edges on their own layer), and writes it, unless `gate`
    /// was cancelled first.
    pub(super) fn run(self, gate: &PublishGate) -> SheetMade {
        let SheetTarget { path, id, format } = self.target;
        // The sheet's own file is replaced, as it is exported again after an
        // edit (Měchenice MECH-2); one saved there while the sheet was
        // projected is kept (review 2026-10-18 MCP-3).
        let replacing = drawing_sheet_target(&path).map_err(SheetRefusal::Failed)?;
        if replacing.as_deref().is_some_and(|sha256| {
            !self.replace && !crate::app::written_drawings::listed(&self.written, &path, sha256)
        }) {
            return Err(SheetRefusal::NotItsOwn);
        }
        let (exported, bytes) =
            drawing_sheet_bytes(&self.snapshot, &self.results, &self.words, id, format)
                .map_err(SheetRefusal::Failed)?;
        if !gate.begin_publish() {
            return Err(SheetRefusal::Cancelled);
        }
        publish_drawing_sheet(&path, &bytes, replacing.as_deref()).map_err(SheetRefusal::Failed)?;
        Ok(exported)
    }
}

/// The answer to a sheet that could not be written because of `error`.
fn sheet_failed(target: &SheetTarget, error: DrawingSheetExportError) -> &'static str {
    let id = target.id;
    match error {
        DrawingSheetExportError::NotFound(_) => failure(
            "drawing_sheet_not_found",
            format!(
                "The document has no drawing sheet {}; inspect action=summary lists its drawing_sheets.",
                id.0
            ),
            json!({"sheet_id": id.0}),
        ),
        DrawingSheetExportError::Evaluating => failure(
            "drawings_unavailable",
            "The exact evaluation is still running; the sheet would draw old solids.",
            json!({"sheet_id": id.0}),
        ),
        DrawingSheetExportError::Projection(_) | DrawingSheetExportError::Export(_) => {
            failed_because("drawings_unavailable", error)
        }
        DrawingSheetExportError::Dwg(_) => failed_because("dwg_conversion_failed", error),
        DrawingSheetExportError::Write(_) => write_failed(error),
    }
}

/// The answer to a sheet for `target` that was not written.
pub(super) fn sheet_refused(target: &SheetTarget, refusal: SheetRefusal) -> &'static str {
    match refusal {
        SheetRefusal::NotItsOwn => not_its_own(&target.path),
        SheetRefusal::Cancelled => "request_cancelled",
        SheetRefusal::Failed(error) => sheet_failed(target, error),
    }
}

/// The answer to the sheet `made` for `target`; a sheet written is noted as
/// the document's.
pub(super) fn finish_sheet(
    app: &mut KetchupApp,
    target: &SheetTarget,
    made: SheetMade,
) -> Result<Value, SheetRefusal> {
    let SheetTarget { path, id, format } = target;
    let sheet = made?;
    app.wrote_drawings([(path.clone(), sheet.sha256.clone())]);
    let (id, format, path) = (*id, *format, path.display().to_string());
    Ok(json!({
        "exported": true,
        "path": path,
        "sheet_id": id.0,
        "sheet": sheet.name,
        "file_format": format.extension(),
        "views": sheet.views,
        "visible_lines": sheet.visible_lines,
        "hidden_lines": sheet.hidden_lines,
        "dxf_curves": dxf_curves(&sheet.curves),
        "annotations": sheet_annotations(&sheet.annotations),
        "bytes": sheet.bytes,
        "basis": "drawing_sheet_exact_projection",
        "dirty": app.is_dirty(),
    }))
}

/// Most exact curves named one by one in an export result; the count is complete.
const NAMED_CURVES: usize = 100;

/// The plan annotations the sheet's top views drew, by kind, and those left
/// out: all of them without a top view, a dimension straight up or down, and by
/// name one that would reach past the border.
fn sheet_annotations(annotations: &SheetAnnotations) -> Value {
    json!({
        "texts": annotations.texts,
        "dimensions": annotations.dimensions,
        "leaders": annotations.leaders,
        "lines": annotations.lines,
        "without_plan_view": annotations.without_plan_view,
        "outside_plan": annotations.outside_plan,
        "off_sheet": annotations.off_sheet.iter().take(NAMED_CURVES).collect::<Vec<_>>(),
        "off_sheet_count": annotations.off_sheet.len(),
    })
}

/// Exact circles written as CIRCLE/ARC, the projected lines they replace, and
/// every exact curve the DXF keeps as lines, named with the reason.
fn dxf_curves(curves: &DxfCurves) -> Value {
    json!({
        "circles": curves.circles,
        "arcs": curves.arcs,
        "replaced_lines": curves.replaced_lines,
        "curves_as_lines": curves.polyline_curves.iter().take(NAMED_CURVES).map(|curve| json!({
            "id": curve.stable_curve_id,
            "curve": curve.curve_kind,
            "reason": curve.reason,
        })).collect::<Vec<_>>(),
        "curves_as_lines_count": curves.polyline_curves.len(),
    })
}

/// The one path policy, shared with the MCP server.
pub(super) use ketchup_mcp::path_policy::local_absolute_path;

/// A project drawings export the window has taken: the settings it keeps in
/// the document and what restores them when no file is written.
pub(super) struct StartedDrawings {
    pub(super) path: String,
    pub(super) file_format: SetFileFormat,
    /// The files there the set may replace, taken before it is drawn.
    targets: SetTargets,
    previous: SheetSettings,
    previously_unsaved: bool,
    /// The project the set is drawn from and its file.
    owner: SetOwner,
    /// The settings as the export left them in the document.
    kept: Option<SheetSettings>,
}

impl StartedDrawings {
    /// Puts back the settings the export kept: nothing was written. Settings
    /// the user changed since, or another document's, stay as they are; they
    /// are kept in the file, outside Undo (review 2026-11-02 MCP-122).
    pub(super) fn undo(&self, app: &mut KetchupApp) {
        if app.document.current().document_id() != self.owner.document
            || app.stored_sheet_settings().ok() != self.kept
        {
            return;
        }
        app.store_sheet_settings(self.previous.clone());
        app.drawings.unsaved = self.previously_unsaved;
    }
}

/// The height of ±0,000 a client names: whole millimetres, or `"auto"`
/// (none kept) for the one found from the model; none for any other value.
fn zero_level(value: &Value) -> Option<Option<i32>> {
    if value == "auto" {
        return Some(None);
    }
    value
        .as_i64()
        .and_then(|height| i32::try_from(height).ok())
        .map(Some)
}

/// The table of storeys a client names: a list of `{finished_floor_mm,
/// structural_mm}` in whole millimetres, or `"auto"` (none kept) to find the
/// storeys from the model; none for any other value.
fn storey_table(value: &Value) -> Option<Vec<StoreyLevels>> {
    if value == "auto" {
        return Some(Vec::new());
    }
    serde_json::from_value::<Vec<StoreyLevels>>(value.clone())
        .ok()
        .filter(|table| !table.is_empty())
}

/// The altitude of ±0,000 a client names: whole millimetres, or `"none"`.
fn zero_altitude(value: &Value) -> Option<Option<i32>> {
    if value == "none" {
        return Some(None);
    }
    value
        .as_i64()
        .and_then(|altitude| i32::try_from(altitude).ok())
        .map(Some)
}

/// The direction of north a client names in degrees, in tenths of a degree
/// as the document keeps it, or `"none"`.
fn north_tenths(value: &Value) -> Option<Option<i32>> {
    if value == "none" {
        return Some(None);
    }
    // The range the drawings window's field keeps.
    let degrees = value
        .as_f64()
        .filter(|degrees| (-360.0..=360.0).contains(degrees))?;
    format!("{:.0}", degrees * 10.0).parse().ok().map(Some)
}

/// The table of storeys as an answer names it, and where its storeys came from.
fn storeys_answer(settings: &SheetSettings) -> Value {
    if settings.storeys.is_empty() {
        json!("auto")
    } else {
        json!(settings.storeys)
    }
}

/// Takes the project set export `request`: keeps its settings in the document
/// and takes what the set is drawn from; [`finish_export`] writes what was
/// drawn from it.
pub(super) fn start_export(
    app: &mut KetchupApp,
    request: Request,
    ui_busy: bool,
) -> Result<(StartedDrawings, ProjectDrawingsJob), &'static str> {
    let Request::ExportDrawings(ExportDrawingsRequest {
        expected,
        path,
        sheet_id: None,
        format,
        title_block,
        plan_only,
        drawing_scale,
        sheet_language,
        replace,
        take_over,
        zero_level_mm,
        storeys,
        zero_altitude_mm,
        north_deg,
    }) = request
    else {
        return Err("invalid_request");
    };
    LiveBridge::guard(app, &expected)?;
    LiveBridge::available(app, ui_busy)?;
    let request = SheetRequest {
        plan_only,
        scale: drawing_scale.map(sheet_scale).transpose()?,
    };
    let path = path.as_str();
    let target = Path::new(path);
    let file_format = SetFileFormat::of_path(target).filter(|_| local_absolute_path(path));
    let Some(file_format) = file_format else {
        return Err(failure(
            "invalid_path",
            "The drawings are written to a local absolute path ending in .pdf (one file of all sheets), or .dxf or .dwg (a file per sheet beside it, numbered: plan-01.dxf, plan-02.dxf, …); no network share, device or stream.",
            json!({"path": path}),
        ));
    };
    let format = format.as_deref().map(sheet_format).transpose()?;
    // Unset keeps the stored language; `auto` stores none, the window's language.
    let language = match sheet_language.as_deref() {
        None => None,
        Some("auto") => Some(None),
        Some(code) => Some(Some(SheetLanguage::from_code(code).ok_or_else(|| {
            failure(
                "invalid_params",
                "sheet_language is cs, sk or en, the language the sheets are titled in, or auto for the window's language.",
                json!({"sheet_language": code}),
            )
        })?)),
    };
    let zero_level = zero_level_mm
        .as_ref()
        .map(|value| {
            zero_level(value).ok_or_else(|| {
                failure(
                    "invalid_params",
                    "zero_level_mm is the height of ±0,000 in whole millimetres of the model, or auto to find it from the model.",
                    json!({"zero_level_mm": value}),
                )
            })
        })
        .transpose()?;
    let storeys = storeys
        .as_ref()
        .map(|value| {
            storey_table(value).ok_or_else(|| {
                failure(
                    "invalid_params",
                    "storeys is a list of {finished_floor_mm, structural_mm} in whole millimetres of the model, one per storey, or auto to find the storeys from the model.",
                    json!({"storeys": value}),
                )
            })
        })
        .transpose()?;
    let zero_altitude_mm = zero_altitude_mm
        .as_ref()
        .map(|value| {
            zero_altitude(value).ok_or_else(|| {
                failure(
                    "invalid_params",
                    "zero_altitude_mm is the altitude of ±0,000 above sea level in whole millimetres (245300 for 245,300 m), or none to write none.",
                    json!({"zero_altitude_mm": value}),
                )
            })
        })
        .transpose()?;
    let north_tenths_deg = north_deg
        .as_ref()
        .map(|value| {
            north_tenths(value).ok_or_else(|| {
                failure(
                    "invalid_params",
                    "north_deg is the direction of north in degrees counter-clockwise from the model's +y, from -360 to 360, or none when it is not known.",
                    json!({"north_deg": value}),
                )
            })
        })
        .transpose()?;
    if app.exact.task.is_some() {
        return Err(failure(
            "drawings_unavailable",
            "The exact evaluation is still running; the sheet would miss parts.",
            json!({}),
        ));
    }
    if let Some(field) = title_block
        .keys()
        .find(|field| !TitleField::EDITABLE.contains(field))
    {
        return Err(failure(
            "invalid_params",
            format!(
                "title_block.{} is filled in by the sheet itself.",
                field.key()
            ),
            json!({}),
        ));
    }
    // The set exported again after an edit replaces its files (Měchenice
    // MECH-2): the PDF the window last wrote at the path, or the sheets of
    // this project's earlier set its list names; one saved there while the
    // set is drawn is kept (review 2026-10-18 MCP-3).
    let owner = app.drawing_set_owner();
    let targets = match project_drawings_targets(target, file_format, &owner, false) {
        Err(ProjectDrawingsError::SetOfAnotherProject { list, .. }) if take_over => {
            if let Some(code) = replace_declined(app, &list) {
                return Err(code);
            }
            project_drawings_targets(target, file_format, &owner, true)
        }
        targets => targets,
    }
    .map_err(export_failed)?;
    let existing = targets.get(target).map(String::as_str);
    if file_format == SetFileFormat::Pdf && kept_unless_asked(app, target, existing, false) {
        if !replace {
            return Err(not_its_own(target));
        }
        if let Some(code) = replace_declined(app, target) {
            return Err(code);
        }
    }
    let previous = app.stored_sheet_settings().map_err(|reason| {
        failure(
            "drawings_unavailable",
            format!(
                "The sheet settings stored in the document do not read ({reason}); they are kept as they are. The user can fill in the title block again in the window."
            ),
            json!({}),
        )
    })?;
    let mut started = StartedDrawings {
        path: path.to_owned(),
        file_format,
        targets,
        previous: previous.clone(),
        previously_unsaved: app.drawings.unsaved,
        owner,
        kept: None,
    };
    let mut settings = previous;
    if let Some(format) = format {
        settings.format = format;
    }
    if let Some(language) = language {
        settings.language = language;
    }
    if let Some(zero_level) = zero_level {
        settings.zero_level_mm = zero_level;
    }
    if let Some(storeys) = storeys {
        settings.storeys = storeys;
    }
    if let Some(zero_altitude_mm) = zero_altitude_mm {
        settings.zero_altitude_mm = zero_altitude_mm;
    }
    if let Some(north_tenths_deg) = north_tenths_deg {
        settings.north_tenths_deg = north_tenths_deg;
    }
    settings.title_block.extend(title_block);
    app.store_sheet_settings(settings);
    started.kept = app.stored_sheet_settings().ok();
    match app.project_drawings_job_for(request) {
        Ok(job) => Ok((started, job)),
        Err(error) => {
            started.undo(app);
            Err(export_failed(error))
        }
    }
}

/// Writes what was `made` for the export `started`, or puts back its
/// settings. The settings stay in the document only with the sheet they were
/// written on, and the sheet only as a new file: one saved at the path while
/// it was projected is kept (review 2026-10-18 MCP-3).
pub(super) fn finish_export(
    app: &mut KetchupApp,
    started: StartedDrawings,
    made: Result<(DrawingSet, Vec<Vec<u8>>), ProjectDrawingsError>,
) -> Result<Value, ProjectDrawingsError> {
    let path = started.path.clone();
    let file_format = started.file_format;
    let mut written = Vec::new();
    let exported = made.and_then(|(set, files)| {
        publish_project_drawings(
            Path::new(&path),
            file_format,
            &files,
            &started.targets,
            &started.owner,
        )
        .map(|paths| {
            written = written_files(
                Path::new(&path),
                file_format,
                &started.owner,
                &paths,
                &files,
            );
            ExportedSet { set, paths }
        })
    });
    app.wrote_drawings(written);
    let exported = match exported {
        Ok(exported) => exported,
        Err(error) => {
            started.undo(app);
            return Err(error);
        }
    };
    let settings = app.sheet_settings();
    let set = &exported.set;
    let sheets = set
        .sheets
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let file = exported.paths.get(index).or(exported.paths.first());
            json!({
                "number": sheet.number,
                "shows": sheet.kind,
                "title": sheet.title,
                "format": sheet.format.name(),
                "views": sheet.views,
                "annotations": sheet.annotations,
                "underlay_lines": sheet.underlay_lines,
                "path": file.map(|file| file.display().to_string()),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "exported": true,
        "path": path,
        "file_format": file_format.extension(),
        "files": exported.paths.iter().map(|file| file.display().to_string()).collect::<Vec<_>>(),
        "scale": format!("1:{}", set.scale),
        "sheet_count": set.sheets.len(),
        "sheets": sheets,
        "annotations": set.annotations(),
        "printed_as_question_mark": set.printed_as_question_mark(),
        "format_setting": settings.format.map_or("auto", SheetFormat::name),
        "sheet_language": settings.language.map_or("auto", SheetLanguage::code),
        "zero_level_mm": settings.zero_level_mm.map_or(json!("auto"), |height| json!(height)),
        // The height `auto` found, or the one set (review 2026-11-06 P3).
        "zero_level_used_mm": set.zero_level_mm,
        // Given, or found from the faces of the model, which a layered floor
        // misleads (review 2026-11-08 MF-241).
        "storeys": storeys_answer(&settings),
        "storeys_from": if settings.storeys.is_empty() { "model" } else { "table" },
        "zero_altitude_mm": settings.zero_altitude_mm.map_or(json!("none"), |altitude| json!(altitude)),
        "north_deg": settings.north_deg().map_or(json!("none"), |north| json!(north)),
        "title_block": settings.title_block,
        "basis": "visible_layers_exact_solids",
        "dirty": app.is_dirty(),
    }))
}

/// The answer to an export of the set that failed with `error`, its cause
/// recorded.
pub(super) fn export_failed(error: ProjectDrawingsError) -> &'static str {
    match error {
        ProjectDrawingsError::Drawing(
            error @ ketchup_manufacturing::project_drawings::ProjectDrawingError::DoesNotFit {
                ..
            },
        ) => failure(
            "drawings_do_not_fit",
            format!("{error}; choose a coarser drawing_scale or a larger format."),
            json!({}),
        ),
        ProjectDrawingsError::Drawing(
            ketchup_manufacturing::project_drawings::ProjectDrawingError::TitleValueDoesNotFit(
                error,
            ),
        ) => failure(
            "title_value_does_not_fit",
            format!("{error}. Shorten the value; it is never cut short."),
            json!({
                "field": error.field.key(),
                "lines": error.lines,
                "characters_that_fit": error.characters,
                "length": error.length,
            }),
        ),
        ProjectDrawingsError::Drawing(error) => failed_because("drawings_unavailable", error),
        ProjectDrawingsError::NotEvaluated(parts) => failure(
            "drawings_unavailable",
            format!(
                "{} visible parts have no exact solid, so the sheet would leave them out: {}. Fix or hide them, wait for the exact evaluation, then export again.",
                parts.len(),
                crate::app::project_drawings::missing_parts_list(&parts)
            ),
            json!({"parts_without_exact_solid": parts}),
        ),
        ProjectDrawingsError::UnresolvedBodies(part, cause) => failure(
            "drawings_unavailable",
            format!(
                "The bodies of {part} do not resolve to one exact solid each ({cause}), so the sheet would leave it out. Give each extrusion its own body, or suppress one, then export again."
            ),
            json!({"part": part}),
        ),
        ProjectDrawingsError::ExactResults(cause) => failed_because("drawings_unavailable", cause),
        ProjectDrawingsError::ProgramPlanning => failure(
            "drawings_unavailable",
            "The program is still being evaluated, so the threads of its parts are not known yet. Export again when it is done.",
            json!({"program_planning": true}),
        ),
        ProjectDrawingsError::SettingsUnreadable(error) => {
            failed_because("drawings_unavailable", error)
        }
        ProjectDrawingsError::CosmeticThreads(part, cause) => failure(
            "drawings_unavailable",
            format!(
                "The modelled threads of {part} could not be drawn by ISO 6410 ({cause}); the sheet would show every edge of their helices. Export again, or switch the threads to cosmetic."
            ),
            json!({"part": part}),
        ),
        ProjectDrawingsError::UnderlayUnreadable(name, cause) => failure(
            "drawings_unavailable",
            format!(
                "The lines of the shown underlay {name} cannot be read ({}), so the plans would leave it out. Import the drawing again, or hide the underlay to draw the plans without it.",
                crate::app::project_drawings::underlay_unreadable_reason(cause.as_ref())
            ),
            json!({"underlay": name}),
        ),
        ProjectDrawingsError::Dwg(error) => failed_because("dwg_conversion_failed", error),
        ProjectDrawingsError::SetOfAnotherProject {
            list,
            project,
            sheets,
        } => {
            let names = crate::app::project_drawings::sheet_names(&sheets);
            let deletes = "deletes those beyond the last sheet of this project's set";
            let has = match sheets.len() {
                0 => "no sheets listed, so taking it over only writes this project's set there"
                    .to_owned(),
                1 => format!("1 sheet, {names}: taking it over replaces it and {deletes}"),
                count => {
                    format!("{count} sheets, {names}: taking it over replaces them and {deletes}")
                }
            };
            failure(
                "drawings_set_of_another_project",
                format!(
                    "The drawing set listed in {} was exported from {}, not from this document, so none of its sheets is replaced. It has {has}. Export to another folder or file name, or export again with take_over: true and the window asks the user whether to take that set over for this project.",
                    list.display(),
                    project.as_deref().unwrap_or("a project not saved"),
                ),
                json!({
                    "list": list.display().to_string(),
                    "exported_from": project,
                    "sheets": sheets.len(),
                    "sheet_files": crate::app::project_drawings::listed_sheets(&sheets),
                }),
            )
        }
        ProjectDrawingsError::Write(error) => write_failed(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// The project set to `path`, drawn and written here.
    fn export(
        app: &mut KetchupApp,
        path: &str,
        format: Option<&str>,
        title_block: BTreeMap<TitleField, String>,
        request: SheetRequest,
    ) -> Result<Value, &'static str> {
        LiveBridge::export_drawings(
            app,
            Request::ExportDrawings(ExportDrawingsRequest {
                expected: None,
                path: path.to_owned(),
                sheet_id: None,
                format: format.map(str::to_owned),
                title_block,
                plan_only: request.plan_only,
                drawing_scale: request.scale.map(f64::from),
                sheet_language: None,
                replace: false,
                take_over: false,
                zero_level_mm: None,
                storeys: None,
                zero_altitude_mm: None,
                north_deg: None,
            }),
            false,
        )
    }

    /// Review 2026-11-07 P3: a list of no sheets was told as "It has 0 sheets,
    /// : taking it over replaces them", and the names a list on a share holds
    /// went to the client however long.
    #[test]
    fn the_answer_for_another_projects_set_says_no_sheets_and_cuts_long_names() {
        use crate::app::project_drawings::SHEET_NAME_CHARS;
        let refused = |sheets: Vec<String>| {
            let code = export_failed(ProjectDrawingsError::SetOfAnotherProject {
                list: PathBuf::from("house.dxf.sheets.txt"),
                project: Some("barn.ketchup".to_owned()),
                sheets,
            });
            assert_eq!(code, "drawings_set_of_another_project");
            crate::live_bridge::take_error_details().unwrap_or_default()
        };
        let none = refused(Vec::new()).to_string();
        assert!(none.contains("It has no sheets listed"), "{none}");
        assert!(!none.contains(", :"), "{none}");
        let long = "a".repeat(10_000);
        let answer = refused(vec![long.clone(), "house-02.dxf".to_owned()]);
        let text = answer.to_string();
        assert!(text.len() < 2_000, "{} characters", text.len());
        assert!(text.contains("house-02.dxf"), "{text}");
        assert!(
            text.contains(&format!("{}…", &long[..SHEET_NAME_CHARS])),
            "{text}"
        );
    }

    #[test]
    fn a_failed_export_keeps_the_file_there_and_not_its_settings() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("sheet.pdf");
        std::fs::write(&target, b"user file").unwrap();
        let dialogs = crate::dialogs::ScriptedFileDialogs::new()
            .queue_refused_high_risk()
            .queue_high_risk_approval(9);
        let probe = dialogs.clone();
        let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
        let before = app.stored_sheet_settings().unwrap();
        let title = BTreeMap::from([(TitleField::Author, "AI".to_owned())]);
        // A file the window did not write is not replaced unless asked
        // (review 2026-11-04 MCP-140).
        assert_eq!(
            export(
                &mut app,
                target.to_str().unwrap(),
                Some("A1"),
                title.clone(),
                SheetRequest::default()
            ),
            Err("drawings_target_exists")
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"user file");
        assert!(probe.high_risk_prompts().is_empty());
        // replace: true asks the user in the window (review 2026-11-08
        // SEC-182); declined, the file is kept. Agreed, it is still kept: the
        // empty document has no exact solid to draw.
        let asked = Request::ExportDrawings(ExportDrawingsRequest {
            expected: None,
            path: target.display().to_string(),
            sheet_id: None,
            format: Some("A1".to_owned()),
            title_block: title,
            plan_only: false,
            drawing_scale: None,
            sheet_language: None,
            replace: true,
            take_over: false,
            zero_level_mm: None,
            storeys: None,
            zero_altitude_mm: None,
            north_deg: None,
        });
        assert_eq!(
            LiveBridge::export_drawings(&mut app, asked.clone(), false),
            Err("drawings_replace_declined")
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"user file");
        assert_eq!(
            LiveBridge::export_drawings(&mut app, asked, false),
            Err("drawings_unavailable")
        );
        assert_eq!(probe.high_risk_prompts().len(), 2);
        assert_eq!(std::fs::read(&target).unwrap(), b"user file");
        assert_eq!(app.stored_sheet_settings().unwrap(), before);
        assert!(!app.drawings.unsaved);
    }

    #[test]
    fn a_failed_export_does_not_keep_its_settings() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("sheet.pdf");
        let mut app = KetchupApp::new();
        let before = app.stored_sheet_settings().unwrap();
        let title = BTreeMap::from([(TitleField::Author, "AI".to_owned())]);
        // The empty document has no exact solid to draw.
        assert!(
            export(
                &mut app,
                target.to_str().unwrap(),
                Some("A1"),
                title,
                SheetRequest::default()
            )
            .is_err()
        );
        assert!(!target.exists());
        assert_eq!(app.stored_sheet_settings().unwrap(), before);
        assert!(!app.drawings.unsaved);
    }

    #[test]
    fn unreadable_stored_settings_are_kept_and_stop_the_export() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("sheet.pdf");
        let mut app = KetchupApp::new();
        let corrupt = b"{\"title_block\": {\"client\": \"J\xc3\xa1n\"".to_vec();
        app.file.container_data.set_extension(
            ketchup_model::persistence::ExtensionEntry::new(
                "org.ketchup.drawings",
                "sheet-v1.json",
                false,
                corrupt.clone(),
            )
            .unwrap(),
        );
        let stored = |app: &KetchupApp| {
            app.file
                .container_data
                .extensions()
                .find(|entry| entry.path() == "sheet-v1.json")
                .map(|entry| entry.bytes().to_vec())
        };
        assert!(app.stored_sheet_settings().is_err());
        let title = BTreeMap::from([(TitleField::Author, "AI".to_owned())]);
        assert!(
            export(
                &mut app,
                target.to_str().unwrap(),
                Some("A1"),
                title,
                SheetRequest::default()
            )
            .is_err()
        );
        assert_eq!(
            stored(&app),
            Some(corrupt.clone()),
            "the stored settings are kept"
        );
        assert!(!target.exists());

        let error = app
            .write_project_drawings(
                &target,
                SetFileFormat::Pdf,
                SheetRequest::default(),
                &Default::default(),
                &AtomicBool::new(false),
            )
            .err()
            .unwrap();
        assert!(matches!(error, ProjectDrawingsError::SettingsUnreadable(_)));
        assert!(!target.exists());
        assert_eq!(stored(&app), Some(corrupt));
    }

    /// Review 2026-10-18 MCP-3: the sheet is written only as a new file, so
    /// one saved at the path while it was projected is kept.
    #[test]
    fn a_sheet_is_published_only_as_a_new_file() {
        let directory = tempfile::tempdir().unwrap();
        let pdf = directory.path().join("sheet.pdf");
        let dxf = directory.path().join("sheet.dxf");
        std::fs::write(&pdf, b"saved meanwhile").unwrap();
        std::fs::write(&dxf, b"saved meanwhile").unwrap();
        assert!(
            crate::app::project_drawings::publish_project_drawings(
                &pdf,
                SetFileFormat::Pdf,
                &[b"%PDF-".to_vec()],
                &Default::default(),
                &crate::app::project_drawings::SetOwner {
                    document: ketchup_model::document::DocumentId(1),
                    file: None,
                    token: None,
                    written: BTreeMap::new(),
                },
            )
            .is_err()
        );
        assert!(
            crate::app::drawing_sheet_export::publish_drawing_sheet(&dxf, b"0\nEOF\n", None)
                .is_err()
        );
        assert_eq!(std::fs::read(&pdf).unwrap(), b"saved meanwhile");
        assert_eq!(std::fs::read(&dxf).unwrap(), b"saved meanwhile");
        assert_eq!(
            refuse_existing(pdf.to_str().unwrap()),
            Err("drawings_target_exists")
        );
    }

    /// Review 2026-10-17 MCP-2: a sheet that could not be drawn, or settings
    /// that did not read, answered a bare code and dropped why (AGENTS.md §2).
    #[test]
    fn a_sheet_that_cannot_be_written_answers_with_its_cause() {
        let unreadable = serde_json::from_slice::<Value>(b"{").unwrap_err();
        let unreadable_reason = unreadable.to_string();
        let empty = ketchup_manufacturing::project_drawings::ProjectDrawingError::Empty;
        for (error, cause) in [
            (ProjectDrawingsError::Drawing(empty), empty.to_string()),
            (
                ProjectDrawingsError::SettingsUnreadable(unreadable),
                unreadable_reason,
            ),
        ] {
            assert_eq!(export_failed(error), "drawings_unavailable");
            let details = crate::live_bridge::take_error_details().unwrap_or_default();
            assert!(
                details.to_string().contains(&cause),
                "{cause} not in {details}"
            );
        }
    }

    /// Review 2026-10-19 P3: a title block value too long for its cell is
    /// refused with the field and what fits, not cut short.
    #[test]
    fn a_title_value_too_long_for_its_cell_answers_with_the_field_and_what_fits() {
        let refused = ketchup_manufacturing::title_block::TitleValueDoesNotFit {
            field: ketchup_manufacturing::title_block::TitleField::DrawingNumber,
            lines: 4,
            characters: 96,
            length: 240,
        };
        assert_eq!(
            export_failed(ProjectDrawingsError::Drawing(refused.into())),
            "title_value_does_not_fit"
        );
        let answer = crate::live_bridge::take_error_details().unwrap_or_default();
        let details = &answer["details"];
        assert_eq!(details["field"], "drawing_number", "{answer}");
        assert_eq!(details["lines"], 4, "{answer}");
        assert_eq!(details["characters_that_fit"], 96, "{answer}");
        assert_eq!(details["length"], 240, "{answer}");
    }

    /// Review 2026-11-04 P3: a file another program put in place while the
    /// sheet was being written was answered "Nothing was written; the file is
    /// as it was", but the file is that program's and the one it replaced is
    /// kept in a hidden `.ketchup-export-backup-*` beside it. The answer names
    /// where that one is.
    #[test]
    fn a_file_replaced_during_the_export_names_where_the_original_is_kept() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sheet.pdf");
        let preserved = directory.path().join(".ketchup-export-backup-x1");
        let error = ExportTransactionError::OriginalPreserved {
            path: path.clone(),
            preserved: preserved.clone(),
        };
        assert_eq!(write_failed(error), "drawings_target_exists");
        let answer = crate::live_bridge::take_error_details().unwrap_or_default();
        let reason = answer["reason"].as_str().unwrap_or_default();
        assert!(!reason.contains("as it was"), "{answer}");
        assert!(
            reason.contains(&preserved.display().to_string()),
            "{answer}"
        );
        assert_eq!(
            answer["details"]["preserved"],
            preserved.display().to_string(),
            "{answer}"
        );
    }
}
