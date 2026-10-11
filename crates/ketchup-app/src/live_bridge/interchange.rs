//! `file action=import` and `file action=export`: the exchange formats of the
//! File menu for a client. Imports run the window's review plan and commit it as
//! one Undo step; exports run the window's exporters. A lossy conversion writes
//! only new files and returns its loss report instead of asking; a release to
//! manufacturing waits for the user's confirmation in the window as after the menu.

use super::{LiveBridge, Request, drawings::local_absolute_path, failure, record_rejection};
use crate::KetchupApp;
use crate::app::import_file::{import_format, parse_unit};
use crate::export_bundle::ExportError;
use ketchup_application::rejections::rejected;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::AtomicBool;

type Exporter = fn(&mut KetchupApp, &Path) -> Result<(), ExportError>;

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
    pub(super) fn import_file(
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::Import {
            expected,
            path,
            unit,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        // The local check comes first: `is_file` on a network path would
        // already authenticate to its server.
        if !local_absolute_path(&path) || !Path::new(&path).is_file() {
            return Err(failure(
                "invalid_path",
                "Import reads an existing file at a local absolute path.",
                json!({"path": path}),
            ));
        }
        let format = import_format(Path::new(&path)).ok_or_else(|| {
            failure(
                "unsupported_format",
                "Import reads .step/.stp, .iges/.igs, .stl, .dxf, .glb and .kscene files.",
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
        Self::require_request_authority(cancelled)?;
        let summary = app
            .import_file(format, Path::new(&path), unit)
            .map_err(|error| {
                record_rejection(
                    &rejected("import_failed").caused_by(error),
                    json!({"path": path}),
                );
                "import_failed"
            })?;
        Ok(json!({"imported": true, "path": path, "review": summary, "dirty": app.is_dirty()}))
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
        let extension = target
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|extension| extensions.contains(&extension.as_str()));
        let Some(extension) = extension.filter(|_| local_absolute_path(&path)) else {
            return Err(failure(
                "invalid_path",
                format!(
                    "{export_format} is written to a local absolute path ending in .{}.",
                    extensions.join(" or .")
                ),
                json!({"path": path}),
            ));
        };
        Self::require_request_authority(cancelled)?;
        app.live.client_export = true;
        let exported = exporter(app, target);
        app.live.client_export = false;
        exported.map_err(|error| {
            record_rejection(
                &rejected("export_failed").caused_by(error),
                json!({"path": path, "export_format": export_format}),
            );
            "export_failed"
        })?;
        let loss_report = target.with_extension(format!("{extension}.loss.txt"));
        Ok(json!({
            "exported": true,
            "path": path,
            "export_format": export_format,
            "loss_report": loss_report.is_file().then(|| loss_report.display().to_string()),
            "basis": "visible_parts",
        }))
    }
}
