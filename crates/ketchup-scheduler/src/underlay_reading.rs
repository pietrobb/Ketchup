//! A drawing read as an underlay in the program worker: the file is read, a DWG converted
//! to DXF, and every line kept and written to the request's output file. The worker's
//! memory and time limits bound it, so a drawing of any size either reads or stops the
//! worker alone.

use std::fmt;
use std::io::{BufReader, Cursor};
use std::path::Path;

use ketchup_model::import::{
    DxfImportError, DxfImportOptions, EditedDxfError, read_dxf_underlay_from, write_edited_dxf,
};
use ketchup_model::underlay::UnderlayLines;

use crate::WorkerError;
use crate::program_worker::PROGRAM_WORKER_MEMORY_BYTES;
use crate::protocol::{
    UnderlayInput, UnderlayOutcome, UnderlayRead, UnderlayRefusal, UnderlayWriteInput,
    UnderlayWriteOutcome, UnderlayWritten,
};

/// Reads `input` in the worker process.
pub(crate) fn read_underlay(input: &UnderlayInput) -> UnderlayOutcome {
    read(input).unwrap_or_else(UnderlayOutcome::Refused)
}

fn read(input: &UnderlayInput) -> Result<UnderlayOutcome, UnderlayRefusal> {
    let options =
        DxfImportOptions::new(input.unit_if_unitless).with_geo_origin(input.geo_origin_mm);
    // A DXF file is read line by line; a DWG drawing is converted whole, and its DXF text
    // read line by line from memory.
    let (underlay, dwg) = if input.dwg {
        let source = std::fs::read(&input.source).map_err(|error| file(&input.source, &error))?;
        let drawing = ketchup_dwg::dwg_to_dxf(&source, usize::MAX)
            .map_err(|error| UnderlayRefusal::Dwg(error.into()))?;
        drop(source);
        let skipped = drawing
            .skipped
            .into_iter()
            .map(|records| (records.record_type, records.count))
            .collect();
        let underlay = read_dxf_underlay_from(Cursor::new(drawing.dxf), options);
        (underlay, Some((drawing.version, skipped)))
    } else {
        let source =
            std::fs::File::open(&input.source).map_err(|error| file(&input.source, &error))?;
        (
            read_dxf_underlay_from(BufReader::new(source), options),
            None,
        )
    };
    let underlay = underlay.map_err(|error| match error {
        DxfImportError::UnitsRequired => UnderlayRefusal::UnitsRequired,
        error => UnderlayRefusal::Dxf {
            report: error.to_string(),
        },
    })?;
    // Lines the document could not read back are not added.
    let lines = underlay
        .lines
        .encode_readable()
        .map_err(|error| UnderlayRefusal::Dxf {
            report: error.to_string(),
        })?;
    std::fs::write(&input.output, lines).map_err(|error| file(&input.output, &error))?;
    Ok(UnderlayOutcome::Read(Box::new(UnderlayRead {
        segments: underlay.lines.segment_count(),
        texts: underlay.lines.text_count(),
        layers: underlay.layers,
        hidden_layers: underlay.hidden_layers,
        units: underlay.units,
        offset_mm: underlay.offset_mm,
        diagnostics: underlay.diagnostics,
        dwg,
    })))
}

/// Writes `input`'s drawing back as DXF with its changes, in the worker process; to an
/// output ending in `.dwg` that DXF written as DWG.
pub(crate) fn write_underlay(input: &UnderlayWriteInput) -> UnderlayWriteOutcome {
    write(input).unwrap_or_else(UnderlayWriteOutcome::Refused)
}

fn write(input: &UnderlayWriteInput) -> Result<UnderlayWriteOutcome, UnderlayRefusal> {
    let source = std::fs::read(&input.source).map_err(|error| file(&input.source, &error))?;
    // A DWG drawing is converted whole, not as the import reads it: what the import does
    // not read is still written back (review 2026-10-29 CD-60).
    let (source, mut skipped) = if input.dwg {
        let drawing = ketchup_dwg::dwg_to_whole_dxf(&source, usize::MAX)
            .map_err(|error| UnderlayRefusal::Dwg(error.into()))?;
        // The DWG is not held while its DXF is written (review 2026-10-29 CD-63).
        drop(source);
        let skipped = drawing
            .skipped
            .into_iter()
            .map(|records| (records.record_type, records.count))
            .collect();
        (drawing.dxf, skipped)
    } else {
        (source, Vec::new())
    };
    let lines = std::fs::read(&input.lines).map_err(|error| file(&input.lines, &error))?;
    let lines =
        UnderlayLines::decode(&lines, input.layers).map_err(|error| UnderlayRefusal::Dxf {
            report: error.to_string(),
        })?;
    let written = write_edited_dxf(
        &source,
        DxfImportOptions::new(input.unit_if_unitless).with_geo_origin(input.geo_origin_mm),
        &lines,
        &input.edits,
    )
    .map_err(|error| match error {
        EditedDxfError::Read(DxfImportError::UnitsRequired) => UnderlayRefusal::UnitsRequired,
        EditedDxfError::Read(error) => UnderlayRefusal::Dxf {
            report: error.to_string(),
        },
        EditedDxfError::Refused(refusal) => UnderlayRefusal::NotWritten(refusal),
    })?;
    let to_dwg = input
        .output
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dwg"));
    // An application's own entity or object a DWG cannot hold is left out and said, not a
    // refusal of the whole drawing (review 2026-10-29 CD-71).
    let written = if to_dwg {
        let dwg = ketchup_dwg::dxf_to_dwg_leaving_out_custom_records(&written)
            .map_err(|error| UnderlayRefusal::Dwg(error.into()))?;
        skipped.extend(
            dwg.left_out
                .into_iter()
                .map(|records| (records.record_type, records.count)),
        );
        dwg.dwg
    } else {
        written
    };
    std::fs::write(&input.output, &written).map_err(|error| file(&input.output, &error))?;
    Ok(UnderlayWriteOutcome::Written(UnderlayWritten {
        bytes: u64::try_from(written.len()).unwrap_or(u64::MAX),
        skipped,
    }))
}

fn file(path: &Path, error: &std::io::Error) -> UnderlayRefusal {
    UnderlayRefusal::File {
        path: path.to_owned(),
        report: error.to_string(),
    }
}

/// Why a drawing was not read as an underlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnderlayReadError {
    /// The drawing was refused.
    Refused(UnderlayRefusal),
    Cancelled,
    /// Reading needed more memory than the worker may commit.
    MemoryLimit,
    /// The worker could not be started or stopped otherwise.
    WorkerFailed(WorkerError),
}

impl fmt::Display for UnderlayReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(formatter),
            Self::Cancelled => formatter.write_str("reading the drawing was cancelled"),
            Self::MemoryLimit => write!(
                formatter,
                "reading the drawing needed more than {} GiB of memory and was stopped",
                PROGRAM_WORKER_MEMORY_BYTES >> 30
            ),
            Self::WorkerFailed(error) => {
                write!(
                    formatter,
                    "the process that reads drawings stopped: {error}"
                )
            }
        }
    }
}

impl std::error::Error for UnderlayReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::WorkerFailed(error) => Some(error),
            _ => None,
        }
    }
}
