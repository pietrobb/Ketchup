//! A drawing imported as an underlay written back as DXF with what was changed of it in
//! Kečup. The program worker reads the file it was imported from again (a DWG converted
//! to DXF) and writes it with the objects taken out left out, the moved ones where they
//! lie and the added ones after the drawing's own; layers, styles, blocks, dimensions,
//! hatches and every object not changed stay as the file had them.

use crate::*;
use ketchup_application::program_isolation::{
    UnderlayWorkerError, UnderlayWriteInput, write_underlay,
};
use ketchup_model::underlay::UnderlayId;
use ketchup_program::CancelToken;

/// Why a drawing was not written back.
#[derive(Debug)]
pub(crate) enum UnderlayExportError {
    /// The document has no such underlay.
    NotFound(UnderlayId),
    /// The drawing was not imported in this window and no source file was given.
    NoSource(UnderlayId),
    /// The document holds no blob of its lines.
    NoLines(UnderlayId),
    /// The lines could not be handed to the worker.
    Io(std::io::Error),
    Worker(UnderlayWorkerError),
}

impl std::fmt::Display for UnderlayExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(formatter, "the document has no underlay {}", id.0),
            Self::NoSource(id) => write!(
                formatter,
                "underlay {} was not imported in this window; name the DXF or DWG file it \
                 was imported from",
                id.0
            ),
            Self::NoLines(id) => write!(
                formatter,
                "the document holds no lines of underlay {}; import it again",
                id.0
            ),
            Self::Io(error) => write!(
                formatter,
                "the drawing's lines were not handed over: {error}"
            ),
            Self::Worker(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for UnderlayExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Worker(error) => Some(error),
            _ => None,
        }
    }
}

impl KetchupApp {
    /// Writes underlay `id` with its changes to `output` as DXF (DWG for a `.dwg`), read
    /// again from `source`
    /// or else from the file it was imported from in this window; gives the bytes written.
    pub(crate) fn export_underlay_dxf_to(
        &mut self,
        id: UnderlayId,
        source: Option<&Path>,
        output: &Path,
    ) -> Result<u64, UnderlayExportError> {
        let snapshot = self.document.current();
        let underlay = snapshot
            .underlay(id)
            .ok_or(UnderlayExportError::NotFound(id))?;
        let source = source
            .map(Path::to_owned)
            .or_else(|| self.file.activity.underlay.sources.get(&id).cloned())
            .ok_or(UnderlayExportError::NoSource(id))?;
        let blobs = self.file.container_data.blobs();
        let blob = blobs
            .get(&underlay.lines_blob())
            .ok_or(UnderlayExportError::NoLines(id))?;
        let directory = tempfile::tempdir().map_err(UnderlayExportError::Io)?;
        let lines = directory.path().join("underlay-lines.bin");
        std::fs::write(&lines, blob.as_slice()).map_err(UnderlayExportError::Io)?;
        write_underlay(
            UnderlayWriteInput {
                dwg: crate::app::dwg_import::is_dwg_path(&source),
                source,
                // A drawing that declares no unit is matched against its lines in each.
                unit_if_unitless: Some(ImportLengthUnit::Millimetre),
                lines,
                edits: underlay.edits.clone(),
                output: output.to_owned(),
            },
            &CancelToken::default(),
        )
        .map_err(UnderlayExportError::Worker)
    }

    /// Tools > Export drawing to DXF or DWG: the selected drawing, with its changes, to a
    /// file of `extension` the user names; the file it was imported from is asked for when
    /// this window did not import it or it is no longer there.
    pub(crate) fn export_selected_underlay(&mut self, extension: &str) {
        let Some(id) = self.selected_underlay() else {
            return;
        };
        let known = self.file.activity.underlay.sources.get(&id);
        let source = if known.is_some_and(|path| path.is_file()) {
            None
        } else {
            match self.choose_dxf_import_path() {
                Some(path) => Some(path),
                None => return,
            }
        };
        let Some(output) = self.choose_export_path(extension) else {
            return;
        };
        let result = self.export_underlay_dxf_to(id, source.as_deref(), &output);
        self.digest = match &result {
            Ok(_) => self.catalog.format(
                "digest-underlay-exported",
                &BTreeMap::from([("path", output.display().to_string())]),
            ),
            Err(error) => self.catalog.format(
                "digest-underlay-export-failed",
                &BTreeMap::from([("reason", error.to_string())]),
            ),
        };
        // The file named is the drawing's source: the next export does not ask again.
        if let (Ok(_), Some(source)) = (result, source) {
            self.file.activity.underlay.sources.insert(id, source);
        }
    }
}
