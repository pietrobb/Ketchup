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

fn export(
    app: &mut KetchupApp,
    path: &str,
    format: Option<&str>,
    title_block: BTreeMap<TitleField, String>,
) -> Result<Value, &'static str> {
    let target = Path::new(path);
    if path.len() > PATH_BYTES
        || path.contains('\0')
        || !target.is_absolute()
        || !local_file_path(path)
        || !target
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    {
        return Err(failure(
            "invalid_path",
            "The drawings are written to a local absolute path ending in .pdf (no network share, device or stream).",
            json!({"path": path}),
        ));
    }
    // No one confirms an overwrite on this path, so an existing file (or link)
    // is never replaced; the window's own export asks before replacing.
    if std::fs::symlink_metadata(target).is_ok() {
        return Err(failure(
            "drawings_target_exists",
            "A file already exists at this path; export the drawings to a new file name.",
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
        let before = app.stored_sheet_settings();
        let title = BTreeMap::from([(TitleField::Author, "AI".to_owned())]);
        assert_eq!(
            export(&mut app, target.to_str().unwrap(), Some("A1"), title),
            Err("drawings_target_exists")
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"user file");
        assert_eq!(app.stored_sheet_settings(), before);
        assert!(!app.drawings.unsaved);
    }

    #[test]
    fn a_failed_export_does_not_keep_its_settings() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("sheet.pdf");
        let mut app = KetchupApp::new();
        let before = app.stored_sheet_settings();
        let title = BTreeMap::from([(TitleField::Author, "AI".to_owned())]);
        // The empty document has no exact solid to draw.
        assert!(export(&mut app, target.to_str().unwrap(), Some("A1"), title).is_err());
        assert!(!target.exists());
        assert_eq!(app.stored_sheet_settings(), before);
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
