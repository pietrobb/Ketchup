//! `file action=import` and `file action=export`: the exchange formats of the
//! File menu for a client. Imports run the window's review plan and commit it as
//! one Undo step; exports run the window's exporters. A lossy conversion writes
//! only new files and returns its loss report instead of asking; a release to
//! manufacturing waits for the user's confirmation in the window as after the menu.

use super::{LiveBridge, Request, drawings::local_absolute_path, failure, record_rejection};
use crate::app::import_file::{import_format, parse_unit};
use crate::export_bundle::{ExportError, Written};
use crate::{ImportError, ImportFormat, ImportLengthUnit, KetchupApp};
use ketchup_application::rejections::rejected;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::AtomicBool;

type Exporter = fn(&mut KetchupApp, &Path) -> Result<Written, ExportError>;

/// Records why the import of `path` failed for the answer.
pub(super) fn import_failed(path: &str, error: ImportError) -> &'static str {
    record_rejection(
        &rejected("import_failed").caused_by(error),
        json!({"path": path}),
    );
    "import_failed"
}

/// Every export format of the File menu with the file extensions it writes.
const EXPORT_FORMATS: [(&str, &[&str], Exporter); 12] = [
    (
        "step",
        &["step", "stp"],
        KetchupApp::export_current_model_step_to,
    ),
    (
        "iges",
        &["iges", "igs"],
        KetchupApp::export_current_model_iges_to,
    ),
    ("stl", &["stl"], KetchupApp::export_current_model_stl_to),
    (
        "3mf",
        &["3mf"],
        KetchupApp::export_current_model_three_mf_to,
    ),
    ("glb", &["glb"], KetchupApp::export_current_model_glb_to),
    ("dxf", &["dxf"], KetchupApp::export_current_profiles_dxf_to),
    ("sheet_metal_dxf", &["dxf"], |app, path| {
        let feature = app.sole_exportable_sheet_metal_feature_id()?;
        app.export_current_sheet_metal_manufacturing_to(feature, path)
    }),
    (
        "viewer",
        &["ketchup-view"],
        KetchupApp::export_current_model_viewer_to,
    ),
    (
        "fabrication_csv",
        &["csv"],
        KetchupApp::export_current_general_fabrication_to,
    ),
    (
        "member_cut_list_csv",
        &["csv"],
        KetchupApp::export_current_member_cut_list_to,
    ),
    (
        "homag_mpr",
        &["mpr"],
        KetchupApp::export_current_model_homag_mpr_to,
    ),
    ("btlx", &["btlx"], KetchupApp::export_current_model_btlx_to),
];

impl LiveBridge {
    /// Imports the file `request` names. `source` is its bytes when a thread has
    /// read them already (a `.dwg` file, see [`super::dwg_import`]).
    pub(super) fn import_file(
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
        source: Option<Vec<u8>>,
    ) -> Result<Value, &'static str> {
        let (path, format, unit) = Self::import_target(app, request, ui_busy)?;
        Self::require_request_authority(cancelled)?;
        let summary = match source {
            Some(source) => app.import_drawing(Path::new(&path), source, unit),
            None => app.import_file(format, Path::new(&path), unit),
        }
        .map_err(|error| import_failed(&path, error))?;
        Ok(json!({"imported": true, "path": path, "review": summary, "dirty": app.is_dirty()}))
    }

    /// The file, format and unit of an import request the window takes now.
    pub(super) fn import_target(
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
    ) -> Result<(String, ImportFormat, Option<ImportLengthUnit>), &'static str> {
        let Request::Import {
            expected,
            path,
            unit,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        // The local check comes first: `is_file` on a network path would
        // already authenticate to its server.
        if !local_absolute_path(path) || !Path::new(path).is_file() {
            return Err(failure(
                "invalid_path",
                "Import reads an existing file at a local absolute path.",
                json!({"path": path}),
            ));
        }
        let format = import_format(Path::new(path)).ok_or_else(|| {
            failure(
                "unsupported_format",
                "Import reads .step/.stp, .iges/.igs, .stl, .dxf, .dwg, .glb and .kscene files.",
                json!({"path": path}),
            )
        })?;
        let unit = unit
            .as_deref()
            .map(|name| {
                parse_unit(name).ok_or_else(|| {
                    failure(
                        "invalid_params",
                        "unit is one of mm, cm, m, in or ft.",
                        json!({"unit": name}),
                    )
                })
            })
            .transpose()?;
        if !app.confirm_live_import_path(Path::new(path)) {
            return Err("import_declined");
        }
        Ok((path.clone(), format, unit))
    }

    pub(super) fn export_file(
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::Export {
            expected,
            path,
            export_format,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        let Some((_, extensions, exporter)) = EXPORT_FORMATS
            .iter()
            .find(|(name, ..)| *name == export_format)
        else {
            return Err(failure(
                "unsupported_format",
                format!(
                    "export_format is one of {}.",
                    EXPORT_FORMATS.map(|(name, ..)| name).join(", ")
                ),
                json!({"export_format": export_format}),
            ));
        };
        let target = Path::new(&path);
        let known_extension = target
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension.to_ascii_lowercase().as_str()));
        if !known_extension || !local_absolute_path(&path) {
            return Err(failure(
                "invalid_path",
                format!(
                    "{export_format} is written to a local absolute path ending in .{}.",
                    extensions.join(" or .")
                ),
                json!({"path": path}),
            ));
        }
        Self::require_request_authority(cancelled)?;
        app.live.client_export = true;
        let exported = exporter(app, target);
        app.live.client_export = false;
        let written = exported.map_err(|error| {
            record_rejection(
                &rejected("export_failed").caused_by(error),
                json!({"path": path, "export_format": export_format}),
            );
            "export_failed"
        })?;
        // The files the exporter wrote, not names guessed from the path
        // (review 2026-10-17 MCP-1).
        let shown = |path: &Path| path.display().to_string();
        Ok(json!({
            "exported": true,
            "path": shown(&written.primary),
            "export_format": export_format,
            "loss_report": written.loss_report.as_deref().map(shown),
            // BTLx names what it cannot carry, such as threads, in its support report.
            "support_report": written.support_report.as_deref().map(shown),
            "companions": written.companions.iter().map(|path| shown(path)).collect::<Vec<_>>(),
            "basis": "visible_parts",
        }))
    }
}
