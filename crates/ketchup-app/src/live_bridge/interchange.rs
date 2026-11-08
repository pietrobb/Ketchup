//! `file action=import` and `file action=export`: the exchange formats of the
//! File menu for a client. Imports run the window's review plan and commit it as
//! one Undo step; exports run the window's exporters. A lossy conversion writes
//! only new files and returns its loss report instead of asking; a release to
//! manufacturing waits for the user's confirmation in the window as after the menu,
//! and writes nothing if the client disconnected before the user confirmed.

use super::{LiveBridge, Request, drawings::local_absolute_path, failure, record_rejection};
use crate::app::import_file::{import_format, parse_unit};
use crate::export_bundle::{ExportError, Written};
use crate::{ImportError, ImportFormat, ImportLengthUnit, KetchupApp};
use ketchup_application::rejections::rejected;
use ketchup_manufacturing::dxf_annotations::MAX_PLAN_SCALE;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

type Exporter = fn(&mut KetchupApp, &Path) -> Result<Written, ExportError>;

/// What a job's thread has read of a file to import, so the window does not
/// read it again.
pub(super) enum ReadAlready {
    Nothing,
    /// The bytes of a `.dwg` file.
    Drawing(Vec<u8>),
    /// The bytes of a STEP or IGES file and what the exact worker read of them.
    Exact(
        Vec<u8>,
        crate::app::exact_import_activity::ExactImportEvidence,
    ),
}

/// Records why the import of `path` failed for the answer.
pub(super) fn import_failed(path: &str, error: ImportError) -> &'static str {
    record_rejection(
        &rejected("import_failed").caused_by(error),
        json!({"path": path}),
    );
    "import_failed"
}

/// Every export format of the File menu with the file extensions it writes.
const EXPORT_FORMATS: [(&str, &[&str], Exporter); 15] = [
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
    ("dae", &["dae"], KetchupApp::export_current_model_dae_to),
    ("dxf", &["dxf"], KetchupApp::export_current_profiles_dxf_to),
    // The same plan as DXF, written as DWG.
    ("dwg", &["dwg"], KetchupApp::export_current_profiles_dxf_to),
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
        &["mpr", "zip"],
        KetchupApp::export_current_model_homag_mpr_to,
    ),
    ("btlx", &["btlx"], KetchupApp::export_current_model_btlx_to),
    (
        "workshop_package",
        &["zip"],
        KetchupApp::export_current_workshop_package_to,
    ),
];

/// The code of why an export was not written, whose hint repairs that cause:
/// every refusal was `export_failed`, a model the format cannot carry as a
/// full disk (review 2026-11-08, the class of CD-72).
pub(super) fn export_code(error: &ExportError) -> &'static str {
    use ketchup_manufacturing::fabrication::GeneralFabricationError;
    use ketchup_rejection::Rejection;
    match error {
        ExportError::NothingToExport { .. }
        | ExportError::Ambiguous { .. }
        | ExportError::UnsupportedContent { .. }
        | ExportError::OccurrenceNotExportable { .. } => "export_refused",
        ExportError::Failed(cause) => {
            // A projection refusal arrives inside the rejection of its step.
            let mut causes = std::iter::successors(
                Some(cause.as_ref() as &(dyn std::error::Error + 'static)),
                |cause| cause.source(),
            );
            causes
                .find_map(|cause| {
                    if let Some(error) = cause.downcast_ref::<GeneralFabricationError>() {
                        return (!matches!(error, GeneralFabricationError::Package(..)))
                            .then_some("export_refused");
                    }
                    match cause.downcast_ref::<Rejection>()?.code() {
                        "side_effect.declined" => Some("export_declined"),
                        "fabrication.mesh_body" => Some("export_refused"),
                        _ => None,
                    }
                })
                .unwrap_or("export_failed")
        }
        ExportError::Destination { .. }
        | ExportError::ChangedDuringConsent
        | ExportError::ArtifactUnverified { .. }
        | ExportError::Transaction(..) => "export_failed",
    }
}

impl LiveBridge {
    /// Imports the file `request` names, from what a job's thread has `read` of
    /// it already (see [`super::dwg_import`]).
    pub(super) fn import_file(
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
        read: ReadAlready,
    ) -> Result<Value, &'static str> {
        let (path, format, unit) = Self::import_target(app, request, ui_busy)?;
        if matches!(
            request,
            Request::Import {
                as_underlay: true,
                ..
            }
        ) {
            let read = crate::app::underlay::read_underlay_file(
                Path::new(&path),
                unit,
                ketchup_model::underlay::placement::geo_origin(&app.document.current()),
                &ketchup_program::CancelToken::default(),
            )
            .map_err(|error| import_failed(&path, error))?;
            return Self::add_underlay(app, request, ui_busy, cancelled, read);
        }
        Self::require_request_authority(cancelled)?;
        let summary = match read {
            ReadAlready::Nothing => app.import_file(format, Path::new(&path), unit),
            ReadAlready::Drawing(source) => app.import_drawing(Path::new(&path), source, unit),
            ReadAlready::Exact(source, evidence) => {
                app.import_exact_read(format, Path::new(&path), source, evidence)
            }
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
            as_underlay,
            revision_of,
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
        if *as_underlay && format != ImportFormat::Dxf {
            return Err(failure(
                "invalid_params",
                "as_underlay imports a .dxf or .dwg drawing.",
                json!({"path": path}),
            ));
        }
        if let Some(of) = revision_of {
            if !*as_underlay {
                return Err(failure(
                    "invalid_params",
                    "revision_of compares a drawing imported as_underlay with an underlay.",
                    json!({"revision_of": of}),
                ));
            }
            let of = ketchup_model::underlay::UnderlayId(*of);
            if app.document.current().underlay(of).is_none() {
                return Err("entity_not_found");
            }
        }
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
        Ok((path.clone(), format, unit))
    }

    pub(super) fn export_file(
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Value, &'static str> {
        let drawing_scale = match &request {
            Request::Export { drawing_scale, .. } => *drawing_scale,
            _ => None,
        };
        let (path, export_format, exporter) = Self::export_target(app, request, ui_busy)?;
        Self::require_request_authority(cancelled)?;
        app.live.client_export = Some(Arc::clone(cancelled));
        let exported = match drawing_scale {
            Some(scale) => app.export_current_plan_dxf_to(Path::new(&path), scale),
            None => exporter(app, Path::new(&path)),
        };
        app.live.client_export = None;
        Self::exported(&path, &export_format, exported)
    }

    /// The path and format of an export request the window takes now.
    pub(super) fn export_target(
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
    ) -> Result<(String, String, Exporter), &'static str> {
        let Request::Export {
            expected,
            path,
            export_format,
            drawing_scale,
        } = request
        else {
            return Err("invalid_request");
        };
        if let Some(scale) = drawing_scale
            && (!matches!(export_format.as_str(), "dxf" | "dwg")
                || !(1.0..=MAX_PLAN_SCALE).contains(&scale))
        {
            return Err(failure(
                "invalid_params",
                format!(
                    "drawing_scale is N of the scale 1:N of a dxf or dwg export, from 1 to {MAX_PLAN_SCALE}."
                ),
                json!({"drawing_scale": scale, "export_format": export_format}),
            ));
        }
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
        Ok((path, export_format, *exporter))
    }

    /// Records why the export of `path` failed for the answer, and its code.
    pub(super) fn export_failed(
        path: &str,
        export_format: &str,
        error: ExportError,
    ) -> &'static str {
        let code = export_code(&error);
        record_rejection(
            &rejected(code).caused_by(error),
            json!({"path": path, "export_format": export_format}),
        );
        code
    }

    /// The answer to an export of `path` as `export_format`.
    pub(super) fn exported(
        path: &str,
        export_format: &str,
        exported: Result<Written, ExportError>,
    ) -> Result<Value, &'static str> {
        let written = exported.map_err(|error| Self::export_failed(path, export_format, error))?;
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
            // A plan DXF/DWG draws the rest and names the parts it could not.
            "omitted_parts": written.omitted_parts,
            "basis": "visible_parts",
        }))
    }
}
