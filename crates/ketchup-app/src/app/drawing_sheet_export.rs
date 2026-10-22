//! A drawing sheet of the document (MCP `create_drawing`, the assembly
//! drawing from a selection) written as DXF or DWG: the exact projection of
//! `ketchup_model::drawing_export`, hidden edges on their own HIDDEN layer,
//! circles the views face as exact CIRCLE and ARC entities. The DWG is that
//! same DXF converted by `ketchup-dwg` to AutoCAD 2018 (AC1032).
//! The project sheet (`project_drawings.rs`) stays PDF only: it
//! hides back edges by painting nearer faces over them, which DXF cannot carry,
//! so its DXF would show every back edge as a visible line.

use crate::export_bundle::{
    ExportTransactionError, export_target_sha256, write_export_artifact_if_unchanged,
};
use crate::*;
use ketchup_dwg::DwgError;
use ketchup_model::drawing::{DrawingError, DrawingSheetId, project_orthographic_drawing};
use ketchup_model::drawing_export::{DrawingExportError, DxfCurves, export_drawing};
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
    pub(crate) bytes: usize,
}

/// Drawing sheet `id` of `snapshot` as `results` project it, as the bytes of
/// a `format` file; needs nothing of the window, so it can run on a thread.
pub(crate) fn drawing_sheet_bytes(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    id: DrawingSheetId,
    format: SheetFileFormat,
) -> Result<(ExportedDrawingSheet, Vec<u8>), DrawingSheetExportError> {
    let sheet = snapshot
        .drawing_sheet(id)
        .ok_or(DrawingSheetExportError::NotFound(id))?;
    let drawing = project_orthographic_drawing(snapshot, results, sheet)
        .map_err(DrawingSheetExportError::Projection)?;
    let bundle = export_drawing(snapshot, &drawing).map_err(DrawingSheetExportError::Export)?;
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
        bytes: bytes.len(),
    };
    Ok((exported, bytes))
}

/// Writes `bytes` to `path` through a temporary file, so a failed write never
/// leaves half a drawing.
pub(crate) fn publish_drawing_sheet(
    path: &Path,
    bytes: &[u8],
) -> Result<(), DrawingSheetExportError> {
    let expected = export_target_sha256(path).map_err(DrawingSheetExportError::Write)?;
    write_export_artifact_if_unchanged(path, bytes, expected.as_deref())
        .map_err(DrawingSheetExportError::Write)
}

impl KetchupApp {
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
        drawing_sheet_bytes(&self.document.current(), &self.exact.results, id, format)
    }

    /// Writes drawing sheet `id` as a `format` file to `path`.
    pub(crate) fn write_drawing_sheet(
        &self,
        id: DrawingSheetId,
        path: &Path,
        format: SheetFileFormat,
    ) -> Result<ExportedDrawingSheet, DrawingSheetExportError> {
        let (exported, bytes) = self.drawing_sheet_file(id, format)?;
        publish_drawing_sheet(path, &bytes)?;
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
        let result = self.write_drawing_sheet(id, path, format);
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
