//! `file action=export_drawings`: the project drawings of the visible layers as
//! a numbered set of sheets (a plan per storey, sections, elevations), one PDF
//! or a DXF or DWG per sheet, with the title block and format kept in the
//! document; with `sheet_id`, a drawing sheet of the document as DXF or DWG.

use super::{LiveBridge, Request, failed_because, failure};
use crate::KetchupApp;
use crate::app::drawing_sheet_export::{DrawingSheetExportError, SheetFileFormat};
use crate::app::project_drawings::{ProjectDrawingsError, SetFileFormat, SheetRequest};
use ketchup_manufacturing::dxf_annotations::MAX_PLAN_SCALE;
use ketchup_manufacturing::title_block::{SheetFormat, TitleField};
use ketchup_model::drawing::DrawingSheetId;
use ketchup_model::drawing_export::{DxfCurves, SheetAnnotations};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

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
        let Request::ExportDrawings {
            expected,
            path,
            sheet_id,
            format,
            title_block,
            plan_only,
            drawing_scale,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        match sheet_id {
            Some(id) => {
                if format.is_some()
                    || !title_block.is_empty()
                    || plan_only
                    || drawing_scale.is_some()
                {
                    return Err(failure(
                        "invalid_params",
                        "format, title_block, plan_only and drawing_scale belong to the project sheet; a drawing sheet keeps its own page, views, scale and title block.",
                        json!({"sheet_id": id}),
                    ));
                }
                export_sheet(app, &path, DrawingSheetId(id))
            }
            None => {
                let request = SheetRequest {
                    plan_only,
                    scale: drawing_scale.map(sheet_scale).transpose()?,
                };
                export(app, &path, format.as_deref(), title_block, request)
            }
        }
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
fn refuse_existing(path: &str) -> Result<(), &'static str> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(failure(
            "drawings_target_exists",
            "A file already exists at this path; export the drawings to a new file name.",
            json!({"path": path}),
        ));
    }
    Ok(())
}

/// Drawing sheet `id` of the document as DXF, or that DXF as an AutoCAD
/// 2018 DWG: exact views, hidden edges on their own layer.
fn export_sheet(app: &KetchupApp, path: &str, id: DrawingSheetId) -> Result<Value, &'static str> {
    let format = SheetFileFormat::of_path(Path::new(path)).filter(|_| local_absolute_path(path));
    let Some(format) = format else {
        return Err(failure(
            "invalid_path",
            "A drawing sheet is written to a local absolute path ending in .dxf or .dwg (no network share, device or stream).",
            json!({"path": path}),
        ));
    };
    refuse_existing(path)?;
    // Written only as a new file: one saved at the path while the sheet was
    // projected is kept (review 2026-10-18 MCP-3).
    let sheet = app
        .write_drawing_sheet(id, Path::new(path), format, None)
        .map_err(|error| match error {
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
            DrawingSheetExportError::Write(_) => refuse_existing(path)
                .err()
                .unwrap_or_else(|| failed_because("drawings_write_failed", error)),
        })?;
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

fn export(
    app: &mut KetchupApp,
    path: &str,
    format: Option<&str>,
    title_block: BTreeMap<TitleField, String>,
    request: SheetRequest,
) -> Result<Value, &'static str> {
    let target = Path::new(path);
    let file_format = SetFileFormat::of_path(target).filter(|_| local_absolute_path(path));
    let Some(file_format) = file_format else {
        return Err(failure(
            "invalid_path",
            "The drawings are written to a local absolute path ending in .pdf (one file of all sheets), or .dxf or .dwg (a file per sheet beside it, numbered: house-01.dxf, house-02.dxf, …); no network share, device or stream.",
            json!({"path": path}),
        ));
    };
    if file_format == SetFileFormat::Pdf {
        refuse_existing(path)?;
    }
    let format = format.map(sheet_format).transpose()?;
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
    let previous = app.stored_sheet_settings().map_err(|reason| {
        failure(
            "drawings_unavailable",
            format!(
                "The sheet settings stored in the document do not read ({reason}); they are kept as they are. The user can fill in the title block again in the window."
            ),
            json!({}),
        )
    })?;
    let previously_unsaved = app.drawings.unsaved;
    let mut settings = previous.clone();
    if let Some(format) = format {
        settings.format = format;
    }
    settings.title_block.extend(title_block);
    app.store_sheet_settings(settings);
    // The settings stay in the document only with the sheet they were written
    // on, and the sheet only as a new file: one saved at the path while it was
    // projected is kept (review 2026-10-18 MCP-3).
    let exported = app
        .write_project_drawings(target, file_format, request, None)
        .map_err(|error| {
            app.store_sheet_settings(previous);
            app.drawings.unsaved = previously_unsaved;
            match error {
                ProjectDrawingsError::Write(_) => refuse_existing(path)
                    .err()
                    .unwrap_or_else(|| drawings_failed(error)),
                error => drawings_failed(error),
            }
        })?;
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
        "title_block": settings.title_block,
        "basis": "visible_layers_exact_solids",
        "dirty": app.is_dirty(),
    }))
}

/// The answer to a sheet that could not be written, its cause recorded.
fn drawings_failed(error: ProjectDrawingsError) -> &'static str {
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
        ProjectDrawingsError::Dwg(error) => failed_because("dwg_conversion_failed", error),
        ProjectDrawingsError::Write(error) => failed_because("drawings_write_failed", error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_file_is_never_replaced_and_its_settings_are_not_kept() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("sheet.pdf");
        std::fs::write(&target, b"user file").unwrap();
        let mut app = KetchupApp::new();
        let before = app.stored_sheet_settings().unwrap();
        let title = BTreeMap::from([(TitleField::Author, "AI".to_owned())]);
        assert_eq!(
            export(
                &mut app,
                target.to_str().unwrap(),
                Some("A1"),
                title,
                SheetRequest::default()
            ),
            Err("drawings_target_exists")
        );
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
            .write_project_drawings(&target, SetFileFormat::Pdf, SheetRequest::default(), None)
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
                None
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
            assert_eq!(drawings_failed(error), "drawings_unavailable");
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
            drawings_failed(ProjectDrawingsError::Drawing(refused.into())),
            "title_value_does_not_fit"
        );
        let answer = crate::live_bridge::take_error_details().unwrap_or_default();
        let details = &answer["details"];
        assert_eq!(details["field"], "drawing_number", "{answer}");
        assert_eq!(details["lines"], 4, "{answer}");
        assert_eq!(details["characters_that_fit"], 96, "{answer}");
        assert_eq!(details["length"], 240, "{answer}");
    }
}
