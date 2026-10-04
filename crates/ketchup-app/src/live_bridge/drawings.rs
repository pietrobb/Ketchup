//! `file action=export_drawings`: the project drawings of the visible layers as
//! a PDF sheet, with the title block and format kept in the document.

use super::{LiveBridge, Request, failed_because, failure};
use crate::KetchupApp;
use crate::app::project_drawings::ProjectDrawingsError;
use ketchup_manufacturing::title_block::{SheetFormat, TitleField};
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
            format,
            title_block,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        export(app, &path, format.as_deref(), title_block)
    }
}

fn export(
    app: &mut KetchupApp,
    path: &str,
    format: Option<&str>,
    title_block: BTreeMap<TitleField, String>,
) -> Result<Value, &'static str> {
    let target = Path::new(path);
    if path.len() > 4096
        || path.contains('\0')
        || !target.is_absolute()
        || !target
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    {
        return Err(failure(
            "invalid_path",
            "The drawings are written to an absolute path ending in .pdf.",
            json!({"path": path}),
        ));
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
    let mut settings = app.stored_sheet_settings();
    if let Some(format) = format {
        settings.format = format;
    }
    settings.title_block.extend(title_block);
    app.store_sheet_settings(settings);
    let sheet = app
        .write_project_drawings(target)
        .map_err(|error| match error {
            ProjectDrawingsError::Drawing(_) => "drawings_unavailable",
            ProjectDrawingsError::Write(error) => failed_because("drawings_write_failed", error),
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
