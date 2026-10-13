//! `file action=export_drawings`: the project drawings of the visible layers as
//! a PDF sheet, with the title block and format kept in the document; with
//! `sheet_id`, a drawing sheet of the document as DXF.

use super::{LiveBridge, Request, failed_because, failure};
use crate::KetchupApp;
use crate::app::drawing_sheet_export::DrawingSheetExportError;
use crate::app::project_drawings::ProjectDrawingsError;
use ketchup_manufacturing::title_block::{SheetFormat, TitleField};
use ketchup_model::drawing::DrawingSheetId;
use ketchup_model::drawing_export::DxfCurves;
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
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        match sheet_id {
            Some(id) => {
                if format.is_some() || !title_block.is_empty() {
                    return Err(failure(
                        "invalid_params",
                        "format and title_block belong to the project sheet; a drawing sheet keeps its own page and title block.",
                        json!({"sheet_id": id}),
                    ));
                }
                export_sheet(app, &path, DrawingSheetId(id))
            }
            None => export(app, &path, format.as_deref(), title_block),
        }
    }
}

fn has_extension(path: &Path, wanted: &str) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted))
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

/// Drawing sheet `id` of the document as DXF: exact views, hidden edges on
/// their own layer.
fn export_sheet(app: &KetchupApp, path: &str, id: DrawingSheetId) -> Result<Value, &'static str> {
    if !local_absolute_path(path) || !has_extension(Path::new(path), "dxf") {
        return Err(failure(
            "invalid_path",
            "A drawing sheet is written to a local absolute path ending in .dxf (no network share, device or stream).",
            json!({"path": path}),
        ));
    }
    refuse_existing(path)?;
    let sheet = app
        .write_drawing_sheet_dxf(id, Path::new(path))
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
            DrawingSheetExportError::Write(_) => failed_because("drawings_write_failed", error),
        })?;
    Ok(json!({
        "exported": true,
        "path": path,
        "sheet_id": id.0,
        "sheet": sheet.name,
        "views": sheet.views,
        "visible_lines": sheet.visible_lines,
        "hidden_lines": sheet.hidden_lines,
        "dxf_curves": dxf_curves(&sheet.curves),
        "bytes": sheet.bytes,
        "basis": "drawing_sheet_exact_projection",
        "dirty": app.is_dirty(),
    }))
}

/// Most exact curves named one by one in an export result; the count is complete.
const NAMED_CURVES: usize = 100;

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

/// Longest drawing target path, in bytes.
const PATH_BYTES: usize = 4096;

/// A path on a local drive: not a network share or device (`\\server\…`,
/// `\\?\…`, `\\.\…`) and no alternate data stream (`file.pdf:stream`).
fn local_file_path(path: &str) -> bool {
    if cfg!(windows) {
        let unc = path.starts_with(r"\\") || path.starts_with("//");
        let stream = path.char_indices().any(|(index, c)| c == ':' && index != 1);
        !unc && !stream
    } else {
        true
    }
}

/// The one path policy of the protocol for `open`, `save_as` and drawings:
/// absolute, bounded and local. It reads nothing from the file system, so a
/// network path is refused before any request reaches its server.
pub(super) fn local_absolute_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= PATH_BYTES
        && !path.contains('\0')
        && Path::new(path).is_absolute()
        && local_file_path(path)
}

fn export(
    app: &mut KetchupApp,
    path: &str,
    format: Option<&str>,
    title_block: BTreeMap<TitleField, String>,
) -> Result<Value, &'static str> {
    let target = Path::new(path);
    if has_extension(target, "dxf") || has_extension(target, "dwg") {
        return Err(failure(
            "project_sheet_pdf_only",
            "The project sheet hides back edges by painting nearer faces over them, which DXF and DWG cannot carry, so it is written only as PDF. A drawing sheet of the document is written as .dxf with sheet_id.",
            json!({"path": path}),
        ));
    }
    if !local_absolute_path(path) || !has_extension(target, "pdf") {
        return Err(failure(
            "invalid_path",
            "The drawings are written to a local absolute path ending in .pdf (no network share, device or stream).",
            json!({"path": path}),
        ));
    }
    refuse_existing(path)?;
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
    // The settings stay in the document only with the sheet they were written on.
    let sheet = app.write_project_drawings(target).map_err(|error| {
        app.store_sheet_settings(previous);
        app.drawings.unsaved = previously_unsaved;
        match error {
            ProjectDrawingsError::Drawing(_) => "drawings_unavailable",
            ProjectDrawingsError::NotEvaluated(parts) => failure(
                "drawings_unavailable",
                format!(
                    "{} visible parts have no exact solid, so the sheet would leave them out: {}. Fix or hide them, wait for the exact evaluation, then export again.",
                    parts.len(),
                    crate::app::project_drawings::missing_parts_list(&parts)
                ),
                json!({"parts_without_exact_solid": parts}),
            ),
            ProjectDrawingsError::SettingsUnreadable(_) => "drawings_unavailable",
            ProjectDrawingsError::Write(error) => failed_because("drawings_write_failed", error),
        }
    })?;
    let settings = app.sheet_settings();
    Ok(json!({
        "exported": true,
        "path": path,
        "format": sheet.format.name(),
        "scale": format!("1:{}", sheet.scale),
        "views": sheet.views,
        "format_setting": settings.format.map_or("auto", SheetFormat::name),
        "title_block": settings.title_block,
        "basis": "visible_layers_exact_solids",
        "dirty": app.is_dirty(),
    }))
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
            export(&mut app, target.to_str().unwrap(), Some("A1"), title),
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
        assert!(export(&mut app, target.to_str().unwrap(), Some("A1"), title).is_err());
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
        assert!(export(&mut app, target.to_str().unwrap(), Some("A1"), title).is_err());
        assert_eq!(
            stored(&app),
            Some(corrupt.clone()),
            "the stored settings are kept"
        );
        assert!(!target.exists());

        let error = app.write_project_drawings(&target).unwrap_err();
        assert!(matches!(error, ProjectDrawingsError::SettingsUnreadable(_)));
        assert!(!target.exists());
        assert_eq!(stored(&app), Some(corrupt));
    }

    #[test]
    fn network_shares_devices_and_streams_are_not_local_files() {
        assert!(local_file_path(r"C:\work\sheet.pdf"));
        if cfg!(windows) {
            for path in [
                r"\\server\share\sheet.pdf",
                r"\\?\C:\sheet.pdf",
                r"\\.\pipe\sheet.pdf",
                r"C:\work\sheet.pdf:hidden.pdf",
            ] {
                assert!(!local_file_path(path), "{path}");
            }
        }
    }
}
