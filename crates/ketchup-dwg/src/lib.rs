//! DWG files through a DXF bridge.
//!
//! The `acadrust` crate (MPL-2.0, used unmodified from crates.io) reads a DWG into DXF bytes,
//! which Ketchup's own DXF import then plans, and writes a Ketchup DXF drawing as DWG. Only
//! bytes cross this API, so no other crate depends on acadrust's types. A panic inside acadrust
//! is caught and returned as [`DwgError::Panicked`] with its message; an error of acadrust is
//! kept as the cause of [`DwgError::Library`].

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::io::{Cursor, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

use acadrust::{
    DwgReadOptions, DwgReader, DwgWriter, DxfError, DxfReader, DxfVersion, DxfWriter, ReadStats,
};

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

/// The DWG version Ketchup writes: AutoCAD 2018.
pub const WRITTEN_VERSION: &str = "AC1032";

/// The largest DWG file Ketchup reads.
pub const MAX_DWG_SOURCE_BYTES: usize = 32 * 1024 * 1024;

/// The DXF sections whose content [`dwg_to_dxf`] writes: the ones a drawing import reads.
/// The others (classes, tables, objects) are written empty; the objects section alone is
/// commonly most of a drawing's DXF text.
pub const DXF_SECTIONS_KEPT: [&str; 3] = ["HEADER", "BLOCKS", "ENTITIES"];

/// The step of a conversion that failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    ReadDwg,
    WriteDxf,
    ReadDxf,
    WriteDwg,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReadDwg => "reading the DWG file",
            Self::WriteDxf => "writing the DWG drawing as DXF",
            Self::ReadDxf => "reading the DXF drawing",
            Self::WriteDwg => "writing the DWG file",
        })
    }
}

#[derive(Debug)]
pub enum DwgError {
    /// The DWG file is larger than the caller reads; nothing was read.
    SourceTooLarge { bytes: usize, limit: usize },
    /// The DXF text of the DWG drawing grew past the caller's limit; writing stopped there.
    DxfTooLarge { limit: usize },
    /// The DWG file's object stream could not be read at all (a damaged header or section
    /// map, or not a DWG file); `diagnostic` is acadrust's first note on it.
    NothingRead { diagnostic: Option<String> },
    /// acadrust refused the file or drawing.
    Library { stage: Stage, cause: DxfError },
    /// acadrust panicked; `message` is the panic's own text.
    Panicked { stage: Stage, message: String },
}

impl fmt::Display for DwgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceTooLarge { bytes, limit } => {
                write!(
                    formatter,
                    "DWG file has {bytes} bytes, more than the {limit} read"
                )
            }
            Self::DxfTooLarge { limit } => write!(
                formatter,
                "the header, blocks and entities of the DWG drawing are more than the {limit} \
                 bytes of DXF a drawing import reads"
            ),
            Self::NothingRead {
                diagnostic: Some(diagnostic),
            } => write!(formatter, "DWG file has no readable objects: {diagnostic}"),
            Self::NothingRead { diagnostic: None } => {
                formatter.write_str("DWG file has no readable objects")
            }
            Self::Library { stage, .. } => write!(formatter, "acadrust failed {stage}"),
            Self::Panicked { stage, message } => {
                write!(formatter, "acadrust panicked {stage}: {message}")
            }
        }
    }
}

impl Error for DwgError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Library { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

/// Object records of one type that a DWG read could not decode and left out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkippedRecords {
    /// acadrust's name or type code of the record, `unknown` when it gave none.
    pub record_type: String,
    pub count: usize,
}

/// A DWG file read as DXF text.
#[derive(Clone, Debug)]
pub struct DwgDrawing {
    pub dxf: Vec<u8>,
    /// The file's version code, for example `AC1032`.
    pub version: String,
    /// What the read left out, most frequent first; empty when the read was complete.
    pub skipped: Vec<SkippedRecords>,
}

/// Reads a DWG file and writes its drawing as DXF text with the content of
/// [`DXF_SECTIONS_KEPT`] only. The size of the file is checked before reading and the DXF size
/// while writing, so a runaway conversion stops at the limit. Object records the reader cannot
/// decode are left out and listed in [`DwgDrawing::skipped`]; a file whose object stream cannot
/// be read at all is refused.
///
/// The reader trusts sizes the file declares: a damaged or hostile file can make it allocate
/// more memory than there is, which aborts the process. Run it in a process of its own.
pub fn dwg_to_dxf(
    dwg: &[u8],
    max_dwg_bytes: usize,
    max_dxf_bytes: usize,
) -> Result<DwgDrawing, DwgError> {
    let bytes = dwg.len();
    if bytes > max_dwg_bytes {
        return Err(DwgError::SourceTooLarge {
            bytes,
            limit: max_dwg_bytes,
        });
    }
    let outcome = guarded(Stage::ReadDwg, || {
        DwgReader::from_stream_with_options(Cursor::new(dwg), DwgReadOptions::failsafe())
            .read_with_stats()
    })?;
    if !outcome.stats.record_stream_read {
        return Err(DwgError::NothingRead {
            diagnostic: outcome.stats.diagnostics.first().map(|d| d.message.clone()),
        });
    }
    let mut out = SectionWriter::new(max_dxf_bytes);
    let written = guarded(Stage::WriteDxf, || {
        DxfWriter::new(&outcome.document).write_to_writer(&mut out)
    });
    let dxf = out.finish();
    let Some(dxf) = dxf else {
        return Err(DwgError::DxfTooLarge {
            limit: max_dxf_bytes,
        });
    };
    written?;
    Ok(DwgDrawing {
        dxf,
        version: outcome.stats.source_version.clone(),
        skipped: skipped_records(&outcome.stats),
    })
}

/// Writes a DXF drawing as a DWG file of [`WRITTEN_VERSION`].
pub fn dxf_to_dwg(dxf: &[u8]) -> Result<Vec<u8>, DwgError> {
    let source = dxf.to_vec();
    let mut document = guarded(Stage::ReadDxf, move || {
        DxfReader::from_reader(Cursor::new(source))?.read()
    })?;
    document.version = DxfVersion::AC1032;
    guarded(Stage::WriteDwg, || DwgWriter::write_to_vec(&document))
}

/// Runs one acadrust step, keeping its error as the cause and turning a panic into
/// [`DwgError::Panicked`] with the panic's message.
fn guarded<T>(stage: Stage, run: impl FnOnce() -> Result<T, DxfError>) -> Result<T, DwgError> {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(result) => result.map_err(|cause| DwgError::Library { stage, cause }),
        Err(payload) => Err(DwgError::Panicked {
            stage,
            message: panic_message(payload),
        }),
    }
}

/// The text a panic was raised with (`panic!` gives a `&str` or a `String`).
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload.downcast_ref::<&str>().map_or_else(
            || String::from("the panic carried no text"),
            |message| (*message).to_owned(),
        ),
    }
}

/// The records a failsafe read skipped, counted by type. Records beyond acadrust's diagnostic
/// limit have no note of their own and are counted as `unknown`, so the total always matches.
fn skipped_records(stats: &ReadStats) -> Vec<SkippedRecords> {
    let mut by_type = std::collections::BTreeMap::<String, usize>::new();
    for diagnostic in stats
        .diagnostics
        .iter()
        .filter(|d| d.record_handle.is_some())
    {
        let record_type = diagnostic.record_type.as_deref().unwrap_or("unknown");
        *by_type.entry(record_type.to_owned()).or_default() += 1;
    }
    let noted: usize = by_type.values().sum();
    if stats.skipped_source_records > noted {
        *by_type.entry(String::from("unknown")).or_default() +=
            stats.skipped_source_records - noted;
    }
    let mut skipped: Vec<SkippedRecords> = by_type
        .into_iter()
        .map(|(record_type, count)| SkippedRecords { record_type, count })
        .collect();
    skipped.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(a.record_type.cmp(&b.record_type))
    });
    skipped
}

/// Where a [`SectionWriter`] is in the DXF text.
enum Place {
    BetweenSections,
    /// After `0 SECTION`, before the `2 <name>` pair; the first pair is held.
    SectionStart(Vec<u8>),
    InSection {
        kept: bool,
    },
}

/// Collects DXF text pair by pair, keeping only the content of [`DXF_SECTIONS_KEPT`], and fails
/// every write once the kept text would grow past `limit`.
struct SectionWriter {
    out: Vec<u8>,
    limit: usize,
    exceeded: bool,
    /// The line being written, up to its newline.
    line: Vec<u8>,
    /// The group code line of the pair being written.
    code: Option<Vec<u8>>,
    place: Place,
}

impl SectionWriter {
    const fn new(limit: usize) -> Self {
        Self {
            out: Vec::new(),
            limit,
            exceeded: false,
            line: Vec::new(),
            code: None,
            place: Place::BetweenSections,
        }
    }

    fn pair(&mut self, code: &[u8], value: &[u8]) {
        let is = |line: &[u8], text: &str| line.trim_ascii().eq_ignore_ascii_case(text.as_bytes());
        let pair = [code, value].concat();
        match std::mem::replace(&mut self.place, Place::BetweenSections) {
            Place::BetweenSections if is(code, "0") && is(value, "SECTION") => {
                self.place = Place::SectionStart(pair);
            }
            Place::BetweenSections => self.emit(&pair),
            Place::SectionStart(start) => {
                // A section without a name pair keeps its content, for the import to refuse.
                let kept = !is(code, "2") || DXF_SECTIONS_KEPT.iter().any(|name| is(value, name));
                self.emit(&start);
                self.emit(&pair);
                self.place = Place::InSection { kept };
            }
            Place::InSection { .. } if is(code, "0") && is(value, "ENDSEC") => self.emit(&pair),
            Place::InSection { kept } => {
                if kept {
                    self.emit(&pair);
                }
                self.place = Place::InSection { kept };
            }
        }
    }

    fn emit(&mut self, bytes: &[u8]) {
        if self.out.len().saturating_add(bytes.len()) > self.limit {
            self.exceeded = true;
        } else {
            self.out.extend_from_slice(bytes);
        }
    }

    /// The kept text, with any unfinished pair as it was written; `None` past the limit.
    fn finish(mut self) -> Option<Vec<u8>> {
        if let Place::SectionStart(start) =
            std::mem::replace(&mut self.place, Place::BetweenSections)
        {
            self.emit(&start);
        }
        let rest = [
            self.code.take().unwrap_or_default(),
            std::mem::take(&mut self.line),
        ]
        .concat();
        self.emit(&rest);
        (!self.exceeded).then_some(self.out)
    }
}

impl Write for SectionWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.exceeded {
            return Err(std::io::Error::other("DXF size limit reached"));
        }
        for piece in bytes.split_inclusive(|byte| *byte == b'\n') {
            self.line.extend_from_slice(piece);
            if piece.ends_with(b"\n") {
                let line = std::mem::take(&mut self.line);
                match self.code.take() {
                    None => self.code = Some(line),
                    Some(code) => self.pair(&code, &line),
                }
            }
        }
        if self.exceeded {
            return Err(std::io::Error::other("DXF size limit reached"));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
