//! A drawing sheet of the document (MCP `create_drawing`, the assembly
//! drawing from a selection) written as DXF: the exact projection of
//! `ketchup_model::drawing_export`, hidden edges on their own HIDDEN layer.
//! The project sheet (`project_drawings.rs`) stays PDF only: it
//! hides back edges by painting nearer faces over them, which DXF cannot carry,
//! so its DXF would show every back edge as a visible line.

use crate::export_bundle::{
    ExportTransactionError, export_target_sha256, write_export_artifact_if_unchanged,
};
use crate::*;
use ketchup_model::drawing::{DrawingError, DrawingSheetId, project_orthographic_drawing};
use ketchup_model::drawing_export::{DrawingExportError, export_drawing};

/// Why a drawing sheet could not be written as DXF.
#[derive(Debug)]
pub(crate) enum DrawingSheetExportError {
    NotFound(DrawingSheetId),
    /// The exact evaluation is still running, so the sheet would draw old solids.
    Evaluating,
    Projection(DrawingError),
    Export(DrawingExportError),
    Write(ExportTransactionError),
}

impl std::fmt::Display for DrawingSheetExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(formatter, "the document has no drawing sheet {}", id.0),
            Self::Evaluating => formatter.write_str("the exact evaluation is still running"),
            Self::Projection(error) => write!(formatter, "the sheet does not project: {error}"),
            Self::Export(error) => write!(formatter, "the sheet does not export: {error}"),
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
            Self::Write(error) => Some(error),
        }
    }
}

/// What was written: the sheet's name, its views and the DXF size.
pub(crate) struct ExportedDrawingSheet {
    pub(crate) name: String,
    pub(crate) views: Vec<String>,
    pub(crate) visible_lines: usize,
    pub(crate) hidden_lines: usize,
    pub(crate) bytes: usize,
}

impl KetchupApp {
    /// The DXF of drawing sheet `id` as the current exact solids project it.
    pub(crate) fn drawing_sheet_dxf(
        &self,
        id: DrawingSheetId,
    ) -> Result<(ExportedDrawingSheet, Vec<u8>), DrawingSheetExportError> {
        if self.exact.task.is_some() {
            return Err(DrawingSheetExportError::Evaluating);
        }
        let snapshot = self.document.current();
        let sheet = snapshot
            .drawing_sheet(id)
            .ok_or(DrawingSheetExportError::NotFound(id))?;
        let drawing = project_orthographic_drawing(&snapshot, &self.exact.results, sheet)
            .map_err(DrawingSheetExportError::Projection)?;
        let bundle =
            export_drawing(&snapshot, &drawing).map_err(DrawingSheetExportError::Export)?;
        let dxf = bundle.dxf().to_vec();
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
            bytes: dxf.len(),
        };
        Ok((exported, dxf))
    }

    /// Writes drawing sheet `id` as DXF to `path` through a temporary file, so
    /// a failed write never leaves half a drawing.
    pub(crate) fn write_drawing_sheet_dxf(
        &self,
        id: DrawingSheetId,
        path: &Path,
    ) -> Result<ExportedDrawingSheet, DrawingSheetExportError> {
        let (exported, dxf) = self.drawing_sheet_dxf(id)?;
        let expected = export_target_sha256(path).map_err(DrawingSheetExportError::Write)?;
        write_export_artifact_if_unchanged(path, &dxf, expected.as_deref())
            .map_err(DrawingSheetExportError::Write)?;
        Ok(exported)
    }

    /// Writes drawing sheet `id` as DXF and reports it in the digest.
    pub(crate) fn export_drawing_sheet_dxf_to(&mut self, id: DrawingSheetId, path: &Path) -> bool {
        let result = self.write_drawing_sheet_dxf(id, path);
        let path = path.display().to_string();
        self.digest = match &result {
            Ok(sheet) => self.catalog.format(
                "digest-exported-drawing-sheet-dxf",
                &BTreeMap::from([("name", sheet.name.clone()), ("path", path)]),
            ),
            Err(error) => self.catalog.format(
                "error-export-drawing-sheet-dxf",
                &BTreeMap::from([("path", path), ("reason", error.to_string())]),
            ),
        };
        result.is_ok()
    }
}
