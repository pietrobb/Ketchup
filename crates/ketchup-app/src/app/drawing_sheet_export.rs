//! A drawing sheet of the document (MCP `create_drawing`, the assembly
//! drawing from a selection) written as DXF or DWG: the exact projection of
//! `ketchup_model::drawing_export`, hidden edges on their own HIDDEN layer,
//! circles the views face as exact CIRCLE and ARC entities. The DWG is that
//! same DXF converted by `ketchup-dwg` to AutoCAD 2018 (AC1032).
//! The project drawings (`project_drawings.rs`) are a set of sheets; their
//! DXF is each sheet's visible lines, made apart from this projection.

use crate::export_bundle::{
    ExportTransactionError, export_target_sha256, write_export_artifact_if_unchanged,
};
use crate::*;
use ketchup_dwg::DwgError;
use ketchup_model::drawing::{
    DrawingError, DrawingSheetId, PartsListWords, project_orthographic_drawing_in,
};
use ketchup_model::drawing_export::{
    DrawingExportError, DxfCurves, SheetAnnotations, export_drawing_dxf,
};
use ketchup_model::exact_product::ExactResultRegistry;

/// The file a drawing sheet is written as.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SheetFileFormat {
    Dxf,
    /// The DXF converted to an AutoCAD 2018 DWG.
    Dwg,
}

impl SheetFileFormat {
    pub(crate) const ALL: [Self; 2] = [Self::Dxf, Self::Dwg];

    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Dxf => "dxf",
            Self::Dwg => "dwg",
        }
    }

    /// The format a path's extension names, in any letter case.
    pub(crate) fn of_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?;
        Self::ALL
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }
}

/// Why a drawing sheet could not be written.
#[derive(Debug)]
pub(crate) enum DrawingSheetExportError {
    NotFound(DrawingSheetId),
    /// The exact evaluation is still running, so the sheet would draw old solids.
    Evaluating,
    Projection(DrawingError),
    Export(DrawingExportError),
    /// The DXF of the sheet did not convert to DWG.
    Dwg(DwgError),
    Write(ExportTransactionError),
}

impl std::fmt::Display for DrawingSheetExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(formatter, "the document has no drawing sheet {}", id.0),
            Self::Evaluating => formatter.write_str("the exact evaluation is still running"),
            Self::Projection(error) => write!(formatter, "the sheet does not project: {error}"),
            Self::Export(error) => write!(formatter, "the sheet does not export: {error}"),
            Self::Dwg(error) => write!(formatter, "the sheet does not convert to DWG: {error}"),
            Self::Write(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DrawingSheetExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotFound(_) | Self::Evaluating => None,
            Self::Projection(error) => Some(error),
            Self::Export(error) => Some(error),
            Self::Dwg(error) => Some(error),
            Self::Write(error) => Some(error),
        }
    }
}

/// What was written: the sheet's name, its views, the projected lines, how
/// the DXF carries curves and the file's size.
pub(crate) struct ExportedDrawingSheet {
    pub(crate) name: String,
    pub(crate) views: Vec<String>,
    pub(crate) visible_lines: usize,
    pub(crate) hidden_lines: usize,
    pub(crate) curves: DxfCurves,
    /// The plan annotations its top views drew, and those left out.
    pub(crate) annotations: SheetAnnotations,
    pub(crate) bytes: usize,
    /// The SHA-256 of the file's bytes.
    pub(crate) sha256: String,
}

/// Drawing sheet `id` of `snapshot` as `results` project it, its parts list
/// worded in `words`, as the bytes of a `format` file; needs nothing of the
/// window, so it can run on a thread.
pub(crate) fn drawing_sheet_bytes(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    words: &PartsListWords,
    id: DrawingSheetId,
    format: SheetFileFormat,
) -> Result<(ExportedDrawingSheet, Vec<u8>), DrawingSheetExportError> {
    let sheet = snapshot
        .drawing_sheet(id)
        .ok_or(DrawingSheetExportError::NotFound(id))?;
    let drawing = project_orthographic_drawing_in(snapshot, results, sheet, words)
        .map_err(DrawingSheetExportError::Projection)?;
    // The DXF alone: the sheet is never written as SVG or PDF (review 2026-11-02 MF-125).
    let bundle = export_drawing_dxf(snapshot, &drawing).map_err(DrawingSheetExportError::Export)?;
    let dxf = bundle.dxf().to_vec();
    let bytes = match format {
        SheetFileFormat::Dxf => dxf,
        SheetFileFormat::Dwg => {
            ketchup_dwg::dxf_to_dwg(&dxf).map_err(DrawingSheetExportError::Dwg)?
        }
    };
    let exported = ExportedDrawingSheet {
        name: sheet.name().to_owned(),
        views: drawing
            .views
            .iter()
            .map(|view| view.kind.stable_name())
            .collect(),
        visible_lines: drawing
            .views
            .iter()
            .map(|view| view.visible_lines.len())
            .sum(),
        hidden_lines: drawing
            .views
            .iter()
            .map(|view| view.hidden_lines.len())
            .sum(),
        curves: bundle.dxf_curves().clone(),
        annotations: bundle.annotations().clone(),
        bytes: bytes.len(),
        sha256: ketchup_model::graph::sha256_hex(&bytes),
    };
    Ok((exported, bytes))
}

/// Writes `bytes` to `path` through a temporary file, so a failed write never
/// leaves half a drawing. `replacing` is the SHA-256 of the file the export
/// may replace, taken before the sheet was drawn (None: no file may be there),
/// so a file saved at `path` while it was drawn is never overwritten.
pub(crate) fn publish_drawing_sheet(
    path: &Path,
    bytes: &[u8],
    replacing: Option<&str>,
) -> Result<(), DrawingSheetExportError> {
    write_export_artifact_if_unchanged(path, bytes, replacing)
        .map_err(DrawingSheetExportError::Write)
}

/// The SHA-256 of the file at `path` an export started now may replace.
pub(crate) fn drawing_sheet_target(path: &Path) -> Result<Option<String>, DrawingSheetExportError> {
    export_target_sha256(path).map_err(DrawingSheetExportError::Write)
}

use super::project_drawings::SHEET_NAMESPACE;

/// Where the document keeps the drawing files it wrote.
const WRITTEN_PATH: &str = "written-v1.json";

/// What a written drawing file is kept under in the document: the SHA-256 of
/// its canonical path, so `c:\X` and `C:\x` name one file and the document
/// carries no absolute path with the user's name (review 2026-11-04 r19
/// SEC-161); a file that is not there has none.
fn written_key(path: &Path) -> std::io::Result<String> {
    std::fs::canonicalize(path)
        .map(|canonical| ketchup_model::graph::sha256_hex(canonical.as_os_str().as_encoded_bytes()))
}

impl KetchupApp {
    /// The drawing files the document wrote: the SHA-256 of each, by
    /// [`written_key`].
    fn written_drawings(&self) -> Result<BTreeMap<String, String>, serde_json::Error> {
        self.file
            .container_data
            .extensions()
            .find(|entry| entry.namespace() == SHEET_NAMESPACE && entry.path() == WRITTEN_PATH)
            .map_or_else(
                || Ok(BTreeMap::new()),
                |entry| serde_json::from_slice(entry.bytes()),
            )
    }

    /// Notes in the document the drawing files the window wrote, each with
    /// its SHA-256, so they are its own after a restart too. A list that does
    /// not read names no file this export could replace unasked, so a new one
    /// takes its place.
    pub(crate) fn wrote_drawings(&mut self, written: impl IntoIterator<Item = (PathBuf, String)>) {
        let before = self.written_drawings();
        let mut kept = before.as_ref().cloned().unwrap_or_default();
        for (path, sha256) in written {
            // Just written, the file is there; one moved away at once is not
            // this document's to replace.
            if let Ok(key) = written_key(&path) {
                kept.insert(key, sha256);
            }
        }
        if before.as_ref().is_ok_and(|before| *before == kept) {
            return;
        }
        let bytes = serde_json::to_vec(&kept).expect("a map of texts is JSON");
        let entry = ketchup_model::persistence::ExtensionEntry::new(
            SHEET_NAMESPACE,
            WRITTEN_PATH,
            false,
            bytes,
        )
        .expect("the drawings namespace and the list's path are valid");
        self.file.container_data.set_extension(entry);
        self.drawings.unsaved = true;
    }

    /// Whether the file at `path`, of SHA-256 `sha256`, is the last drawing
    /// file the document wrote there (review 2026-11-04 MCP-140), however the
    /// path is spelled (r19 SEC-161). A path or list that does not read names
    /// no such file, so the client asks the user.
    pub(crate) fn wrote_drawing(&self, path: &Path, sha256: &str) -> bool {
        written_key(path).is_ok_and(|key| {
            self.written_drawings()
                .is_ok_and(|written| written.get(&key).is_some_and(|kept| kept == sha256))
        })
    }

    /// The DXF of drawing sheet `id` as the current exact solids project it.
    #[cfg(test)]
    pub(crate) fn drawing_sheet_dxf(
        &self,
        id: DrawingSheetId,
    ) -> Result<(ExportedDrawingSheet, Vec<u8>), DrawingSheetExportError> {
        self.drawing_sheet_file(id, SheetFileFormat::Dxf)
    }

    /// Drawing sheet `id` as the bytes of a `format` file.
    pub(crate) fn drawing_sheet_file(
        &self,
        id: DrawingSheetId,
        format: SheetFileFormat,
    ) -> Result<(ExportedDrawingSheet, Vec<u8>), DrawingSheetExportError> {
        if self.exact.task.is_some() {
            return Err(DrawingSheetExportError::Evaluating);
        }
        drawing_sheet_bytes(
            &self.document.current(),
            &self.exact.results,
            &self.parts_list_words(),
            id,
            format,
        )
    }

    /// The parts list's words in the window's language.
    pub(crate) fn parts_list_words(&self) -> PartsListWords {
        PartsListWords {
            mirrored: self.catalog.text("drawing-parts-list-mirrored"),
            quantity: self.catalog.text("drawing-parts-list-quantity"),
        }
    }

    /// Writes drawing sheet `id` as a `format` file to `path`, replacing only
    /// the file whose SHA-256 is `replacing` (None: only a new file).
    pub(crate) fn write_drawing_sheet(
        &self,
        id: DrawingSheetId,
        path: &Path,
        format: SheetFileFormat,
        replacing: Option<&str>,
    ) -> Result<ExportedDrawingSheet, DrawingSheetExportError> {
        let (exported, bytes) = self.drawing_sheet_file(id, format)?;
        publish_drawing_sheet(path, &bytes, replacing)?;
        Ok(exported)
    }

    /// Writes drawing sheet `id` as a `format` file and reports it in the digest.
    #[cfg(test)]
    pub(crate) fn export_drawing_sheet_to(
        &mut self,
        id: DrawingSheetId,
        path: &Path,
        format: SheetFileFormat,
    ) -> bool {
        let result = drawing_sheet_target(path)
            .and_then(|replacing| self.write_drawing_sheet(id, path, format, replacing.as_deref()));
        self.report_drawing_sheet(path, format, &result);
        result.is_ok()
    }

    pub(crate) fn report_drawing_sheet(
        &mut self,
        path: &Path,
        format: SheetFileFormat,
        result: &Result<ExportedDrawingSheet, DrawingSheetExportError>,
    ) {
        let path = path.display().to_string();
        let exported_key = match format {
            SheetFileFormat::Dxf => "digest-exported-drawing-sheet-dxf",
            SheetFileFormat::Dwg => "digest-exported-drawing-sheet-dwg",
        };
        self.digest = match result {
            Ok(sheet) => self.catalog.format(
                exported_key,
                &BTreeMap::from([("name", sheet.name.clone()), ("path", path)]),
            ),
            Err(error) => self.catalog.format(
                "error-export-drawing-sheet",
                &BTreeMap::from([("path", path), ("reason", error.to_string())]),
            ),
        };
    }
}
