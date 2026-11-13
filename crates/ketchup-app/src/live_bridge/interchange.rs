//! `file action=import` and `file action=export`: the exchange formats of the
//! File menu for a client. Imports run the window's review plan and commit it as
//! one Undo step; exports run the window's exporters. A lossy conversion writes
//! only new files and returns its loss report instead of asking; a release to
//! manufacturing waits for the user's confirmation in the window as after the menu,
//! and writes nothing if the client disconnected before the user confirmed.

use super::{LiveBridge, Request, drawings::local_absolute_path, failure, record_rejection};
use crate::app::import_file::{import_format, parse_unit};
use crate::app::presentation_export::PresentationRequest;
use crate::export_bundle::{ExportError, Written};
use crate::{ImportError, ImportFormat, ImportLengthUnit, KetchupApp};
use ketchup_application::rejections::rejected;
use ketchup_manufacturing::dxf_annotations::MAX_PLAN_SCALE;
use ketchup_manufacturing::presentation::{
    Background, MAX_LABEL_CHARS, MAX_PRESENTATION_LABELS, MAX_PRESENTATION_SAMPLES,
    MAX_PRESENTATION_SIDE, PresentationLabel, check_labels, presentation_supersample,
};
use ketchup_model::import::DxfCodePage;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

type Exporter = fn(&mut KetchupApp, &Path) -> Result<Written, ExportError>;

/// What a job's thread has read of a file to import, so the window does not
/// read it again.
pub(super) enum ReadAlready {
    Nothing,
    /// The bytes of a `.dwg` file.
    Drawing(Vec<u8>),
    /// The bytes of a `.skp` file.
    Model(Vec<u8>),
    /// The bytes of a STEP or IGES file and what the exact worker read of them.
    Exact(
        Vec<u8>,
        crate::app::exact_import_activity::ExactImportEvidence,
    ),
}

/// What an import request reads: the file, its format, the unit of a unitless drawing
/// and the code page an underlay drawing is read in.
pub(super) struct ImportTarget {
    pub(super) path: String,
    pub(super) format: ImportFormat,
    pub(super) unit: Option<ImportLengthUnit>,
    pub(super) code_page: Option<DxfCodePage>,
    /// `code_page` is that of the underlay the drawing is a revision of.
    pub(super) code_page_inherited: bool,
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
const EXPORT_FORMATS: [(&str, &[&str], Exporter); 16] = [
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
    // Made only as a job on a thread of its own (`exchange_export::presents`); review 2026-11-13
    // P3: a synchronous copy here was taken by the tests alone.
    ("presentation_png", &["png"], |_app, _path| {
        Err(ExportError::failed(
            "a presentation image is made on a thread of its own",
        ))
    }),
];

/// How a client's presentation image looks; what it leaves out is as the window shows first.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationImage {
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    /// `white`, `transparent` or `#rrggbb`.
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    shadows: Option<bool>,
    #[serde(default)]
    ambient_occlusion: Option<bool>,
    #[serde(default)]
    outlines: Option<bool>,
    #[serde(default)]
    dimensions: Option<bool>,
    #[serde(default)]
    part_labels: Option<bool>,
    #[serde(default)]
    labels: Vec<ImageLabel>,
    /// Percent opaque (0–100) of the parts of each named material.
    #[serde(default)]
    opacity_percent: BTreeMap<String, u8>,
}

/// A client's text at a model point, in millimetres.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageLabel {
    text: String,
    at_mm: [f64; 3],
}

/// Which `image` field a client gave a value it cannot have, and that value.
pub(super) struct InvalidImage {
    reason: &'static str,
    details: Value,
}

impl PresentationImage {
    fn request(self) -> Result<PresentationRequest, InvalidImage> {
        let mut request = PresentationRequest::default();
        let options = &mut request.options;
        let background = match self.background.as_deref() {
            None => options.background,
            Some("white") => Background::White,
            Some("transparent") => Background::Transparent,
            Some(color) => Background::Color(hex_color(color).ok_or_else(|| InvalidImage {
                reason: "image.background is white, transparent or a colour #rrggbb.",
                details: json!({"background": color}),
            })?),
        };
        options.background = background;
        options.width = self.width.unwrap_or(options.width);
        options.height = self.height.unwrap_or(options.height);
        // Review 2026-11-12 R1112-IMG-501: a size the renderer refuses was export_failed,
        // asking to lower an edge smoothing no one can set.
        presentation_supersample(options.width, options.height, options.supersample).map_err(
            |error| InvalidImage {
                reason: "image.width × image.height is at most max_pixels, each side at most max_side; make the image smaller.",
                details: json!({
                    "width": options.width,
                    "height": options.height,
                    "max_side": MAX_PRESENTATION_SIDE,
                    "max_pixels": MAX_PRESENTATION_SAMPLES,
                    "refused": error.to_string(),
                }),
            },
        )?;
        options.shadows = self.shadows.unwrap_or(options.shadows);
        options.ambient_occlusion = self.ambient_occlusion.unwrap_or(options.ambient_occlusion);
        options.outlines = self.outlines.unwrap_or(options.outlines);
        options.dimensions = self.dimensions.unwrap_or(options.dimensions);
        request.part_labels = self.part_labels.unwrap_or(request.part_labels);
        request.labels = self
            .labels
            .into_iter()
            .map(|ImageLabel { text, at_mm }| PresentationLabel { text, at_mm })
            .collect();
        check_labels(&request.labels).map_err(|error| InvalidImage {
            reason: "image.labels holds single-line texts at finite points, as many and as long as details allow.",
            details: json!({
                "labels": error.to_string(),
                "max_labels": MAX_PRESENTATION_LABELS,
                "max_label_chars": MAX_LABEL_CHARS,
            }),
        })?;
        if let Some((name, percent)) = self
            .opacity_percent
            .iter()
            .find(|(_, percent)| **percent > 100)
        {
            return Err(InvalidImage {
                reason: "image.opacity_percent gives each material 0 (invisible) to 100 (opaque).",
                details: json!({"material": name, "opacity_percent": percent}),
            });
        }
        request.opacity_percent = self.opacity_percent;
        Ok(request)
    }
}

/// How a client's presentation image looks: as `image` says, or as the window shows first.
pub(super) fn presentation_request(
    image: Option<PresentationImage>,
) -> Result<PresentationRequest, InvalidImage> {
    image.map_or_else(
        || Ok(PresentationRequest::default()),
        PresentationImage::request,
    )
}

impl InvalidImage {
    /// The answer to a client's image that cannot be.
    pub(super) fn refused(self) -> &'static str {
        failure("invalid_params", self.reason, self.details)
    }
}

/// `#rrggbb` as its three channels.
fn hex_color(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#').filter(|digits| digits.len() == 6)?;
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

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
                        "fabrication.mesh_body"
                        | "presentation.material"
                        | "presentation.image" => Some("export_refused"),
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

/// An export request the window takes now.
pub(super) struct ExportTarget {
    pub(super) path: String,
    pub(super) export_format: String,
    pub(super) exporter: Exporter,
    /// Replace a file and loss report Kečup wrote at `path`, unchanged since.
    pub(super) overwrite: bool,
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
        let ImportTarget {
            path,
            format,
            unit,
            code_page,
            code_page_inherited,
        } = Self::import_target(app, request, ui_busy)?;
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
                code_page,
                code_page_inherited,
                &ketchup_program::CancelToken::default(),
            )
            .map_err(|error| import_failed(&path, error))?;
            return Self::add_underlay(app, request, ui_busy, cancelled, read);
        }
        Self::require_request_authority(cancelled)?;
        let summary = match read {
            ReadAlready::Nothing => app.import_file(format, Path::new(&path), unit),
            ReadAlready::Drawing(source) => app.import_drawing(Path::new(&path), source, unit),
            ReadAlready::Model(source) => app.import_glb_source(Path::new(&path), source),
            ReadAlready::Exact(source, evidence) => {
                app.import_exact_read(format, Path::new(&path), source, evidence)
            }
        }
        .map_err(|error| import_failed(&path, error))?;
        Ok(json!({"imported": true, "path": path, "review": summary, "dirty": app.is_dirty()}))
    }

    /// The file, format, unit and code page of an import request the window takes now.
    pub(super) fn import_target(
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
    ) -> Result<ImportTarget, &'static str> {
        let Request::Import {
            expected,
            path,
            unit,
            as_underlay,
            revision_of,
            code_page,
            ..
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
        // The code page an underlay drawing is read in: the one the request names, else
        // that of the underlay it is a revision of (review 2026-11-09 UL-303), which one of
        // DXF 2007 or later ignores (review 2026-11-10 UL-400).
        let code_page_inherited = code_page.is_none();
        let code_page = match code_page {
            None => revision_of.and_then(|of| {
                app.document
                    .current()
                    .underlay(ketchup_model::underlay::UnderlayId(of))
                    .and_then(|under| under.code_page)
            }),
            Some(name) if !*as_underlay => {
                return Err(failure(
                    "invalid_params",
                    "code_page reads a drawing imported as_underlay.",
                    json!({"code_page": name}),
                ));
            }
            Some(name) => Some(DxfCodePage::parse(name).ok_or_else(|| {
                failure(
                    "invalid_params",
                    "code_page is the $DWGCODEPAGE name of a code page, such as ANSI_1250.",
                    json!({
                        "code_page": name,
                        "code_pages": DxfCodePage::ALL.map(DxfCodePage::name),
                    }),
                )
            })?),
        };
        Ok(ImportTarget {
            path: path.clone(),
            format,
            unit,
            code_page,
            code_page_inherited,
        })
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
        let ExportTarget {
            path,
            export_format,
            exporter,
            overwrite,
        } = Self::export_target(app, request, ui_busy)?;
        Self::require_request_authority(cancelled)?;
        app.live.client_export = Some(Arc::clone(cancelled));
        app.live.client_overwrite = overwrite;
        let exported = match drawing_scale {
            Some(scale) => app.export_current_plan_dxf_to(Path::new(&path), scale),
            None => exporter(app, Path::new(&path)),
        };
        app.live.client_export = None;
        app.live.client_overwrite = false;
        Self::exported(&path, &export_format, exported)
    }

    /// The path and format of an export request the window takes now.
    pub(super) fn export_target(
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
    ) -> Result<ExportTarget, &'static str> {
        let Request::Export {
            expected,
            path,
            export_format,
            drawing_scale,
            image,
            overwrite,
        } = request
        else {
            return Err("invalid_request");
        };
        if image.is_some() && export_format != "presentation_png" {
            return Err(failure(
                "invalid_params",
                "image describes the picture of a presentation_png export.".to_owned(),
                json!({"export_format": export_format}),
            ));
        }
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
        Ok(ExportTarget {
            path,
            export_format,
            exporter: *exporter,
            overwrite,
        })
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
            // A machine file names what it leaves unmade, such as the pin
            // holes in a bought part.
            "lost": written.lost,
            "basis": "visible_parts",
        }))
    }
}
