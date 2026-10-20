//! Typed exact-worker protocol.
//!
//! The scheduler and the worker are one build. Every message is one CBOR-encoded
//! [`WorkerRequest`] or [`WorkerReply`] behind a little-endian `u32` byte length.
//! One handshake compares [`protocol_identity`]; there are no per-message versions,
//! so adding or changing a field is a change of the type alone.
use crate::{
    CamSimulationWireEvidence, CamSimulationWireRequest, ExactVolumeMeshWireOptions,
    StepAssemblyManifest, StepXdeWorkerEvidence, WorkerExactBRepGraphResult,
};
use ketchup_geometry::contact_patch::ContactPatch;
use ketchup_model::exact_brep_graph::{ExactBRepGraph, MAX_EXACT_BREP_GRAPH_BYTES};
use ketchup_model::graph::sha256_hex;
use ketchup_program::{
    Evaluated, ExactShapes, ProgramError, ProgramModel, Refined, Report, SourceLines,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

/// A maximal graph plus bounded request metadata (CAM motions, source paths): the largest
/// request the exact worker reads. The program worker reads any frame, under its memory limit.
pub const MAX_REQUEST_FRAME_BYTES: usize = MAX_EXACT_BREP_GRAPH_BYTES + 1024 * 1024;
/// Bulky meshes travel through files named in the request, not through replies.
pub const MAX_REPLY_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Digest of the sources that define every wire type, shared by both sides of one build.
pub fn protocol_identity() -> &'static str {
    static IDENTITY: OnceLock<String> = OnceLock::new();
    IDENTITY.get_or_init(|| {
        sha256_hex(
            concat!(
                env!("CARGO_PKG_VERSION"),
                include_str!("protocol.rs"),
                include_str!("lib.rs")
            )
            .as_bytes(),
        )
    })
}

/// An imported source file the worker re-verifies by SHA-256 before reading.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SourceFile {
    pub sha256: String,
    pub path: PathBuf,
}

/// A graph with the imported sources its `ImportedExact` nodes reference, in node order.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphInput {
    pub graph: ExactBRepGraph,
    pub sources: Vec<SourceFile>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum WorkerRequest {
    Hello,
    EvaluateGraph(GraphInput),
    TessellateGraph {
        input: GraphInput,
        result_fingerprint: String,
        output: PathBuf,
    },
    VolumeMeshGraph {
        input: GraphInput,
        result_fingerprint: String,
        options: ExactVolumeMeshWireOptions,
        output: PathBuf,
    },
    ExportGraphStep {
        input: GraphInput,
        result_fingerprint: String,
        output: PathBuf,
    },
    SimulateCam {
        graph: ExactBRepGraph,
        request: Box<CamSimulationWireRequest>,
    },
    InspectStepXde(SourceFile),
    InspectStepPart {
        source: SourceFile,
        part_index: Option<u32>,
    },
    TessellateStepPart {
        source: SourceFile,
        part_index: Option<u32>,
        output: PathBuf,
    },
    ExportStepXdePart {
        source: SourceFile,
        part_index: u32,
        result_fingerprint: String,
        output: PathBuf,
    },
    ConvertStepXdeToIges {
        source: SourceFile,
        output: PathBuf,
    },
    InspectIgesXde(SourceFile),
    InspectIgesPart {
        source: SourceFile,
        part_index: Option<u32>,
    },
    TessellateIgesPart {
        source: SourceFile,
        part_index: Option<u32>,
        output: PathBuf,
    },
    AssembleStep {
        manifest: StepAssemblyManifest,
        sources: Vec<PathBuf>,
        output: PathBuf,
    },
    /// Starts a batch of pair queries; loaded graphs are addressed by slot.
    PairBegin,
    PairLoad(GraphInput),
    PairQuery(PairQuery),
    PairEnd,
    /// Runs a user's rule program. Only the program worker gets it: a process of
    /// its own in a job with a memory limit, killed on cancel and at the deadline.
    EvaluateProgram(ProgramInput),
    /// Checks an evaluated program (collisions, relations, loads, bill of
    /// materials). Only the program worker gets it: on a large model this costs
    /// far more than the evaluation and has no bound of its own (review PRG-1).
    ReportProgram(Box<EvaluatedProgram>),
    /// Re-derives a report's issues and relations with exact answers: it costs
    /// as much as the checks of the report (review 2026-10-15 N-4).
    RefineReport(Box<RefineInput>),
    /// Reads a DWG file as DXF. Only the program worker gets it, for the same
    /// reason: the reader allocates what a damaged file declares.
    ConvertDwg(DwgInput),
    /// Test hooks: a native exception, a long job and a hard crash.
    Exception,
    Sleep {
        milliseconds: u64,
    },
    Crash,
}

/// Row-major local-to-world affine placements of two loaded slots.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairQuery {
    pub left: usize,
    pub right: usize,
    pub tolerance_mm: f64,
    /// Zero-based face ordinals on evaluated local bodies; None retains solid-pair semantics.
    pub faces: Option<[u32; 2]>,
    pub left_transform: [f64; 16],
    pub right_transform: [f64; 16],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum WorkerReply {
    Hello {
        protocol: String,
    },
    Done,
    Graph(WorkerExactBRepGraphResult),
    Mesh(MeshReceipt),
    VolumeMesh(VolumeMeshReceipt),
    Exported(ExportReceipt),
    Cam(CamSimulationWireEvidence),
    Xde(StepXdeWorkerEvidence),
    ImportPart(WorkerImportPart),
    PairLoaded {
        slot: usize,
    },
    Pair(PairMeasure),
    Program(Box<ProgramOutcome>),
    Report(Box<ReportOutcome>),
    Refined(Box<RefineOutcome>),
    Dwg(Box<DwgOutcome>),
    Failure(WorkerFailure),
    /// The result did not fit one reply frame; the worker stays usable.
    TooLarge {
        faces: Option<u64>,
    },
}

impl WorkerReply {
    /// What the worker answers instead of this reply when it does not fit one frame.
    pub fn too_large(&self) -> Self {
        let faces = |counts: &[u32; 5]| u64::from(counts[2]);
        Self::TooLarge {
            faces: match self {
                Self::Graph(result) => Some(faces(&result.topology_counts)),
                Self::ImportPart(part) => Some(faces(&part.topology_counts)),
                Self::Xde(evidence) => Some(
                    evidence
                        .parts
                        .iter()
                        .map(|part| faces(&part.topology_counts))
                        .sum(),
                ),
                _ => None,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DwgInput {
    #[serde(with = "cbor_bytes")]
    pub dwg: Vec<u8>,
    pub max_dxf_bytes: usize,
}

/// [`ketchup_dwg::dwg_to_dxf`] on the wire.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum DwgOutcome {
    Converted {
        #[serde(with = "cbor_bytes")]
        dxf: Vec<u8>,
        version: String,
        /// Record type and count of every kind of record the read left out.
        skipped: Vec<(String, usize)>,
    },
    Refused(DwgRefusal),
}

/// A [`ketchup_dwg::DwgError`] on the wire, with its facts as fields.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DwgRefusal {
    DxfTooLarge {
        limit: usize,
    },
    NothingRead {
        diagnostic: Option<String>,
    },
    /// acadrust refused the file or panicked reading it: the error's report
    /// with its cause, which does not cross the process boundary as a type.
    Reader {
        report: String,
    },
}

impl From<ketchup_dwg::DwgError> for DwgRefusal {
    fn from(error: ketchup_dwg::DwgError) -> Self {
        use ketchup_dwg::DwgError;
        match error {
            DwgError::DxfTooLarge { limit } => Self::DxfTooLarge { limit },
            DwgError::NothingRead { diagnostic } => Self::NothingRead { diagnostic },
            DwgError::Library { .. } | DwgError::Panicked { .. } => {
                let report = std::error::Error::source(&error)
                    .map_or_else(|| error.to_string(), |cause| format!("{error}: {cause}"));
                Self::Reader { report }
            }
        }
    }
}

impl std::fmt::Display for DwgRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ketchup_dwg::DwgError;
        match self {
            Self::DxfTooLarge { limit } => DwgError::DxfTooLarge { limit: *limit }.fmt(formatter),
            Self::NothingRead { diagnostic } => DwgError::NothingRead {
                diagnostic: diagnostic.clone(),
            }
            .fmt(formatter),
            Self::Reader { report } => formatter.write_str(report),
        }
    }
}

impl std::error::Error for DwgRefusal {}

impl From<Result<ketchup_dwg::DwgDrawing, ketchup_dwg::DwgError>> for DwgOutcome {
    fn from(result: Result<ketchup_dwg::DwgDrawing, ketchup_dwg::DwgError>) -> Self {
        match result {
            Ok(drawing) => Self::Converted {
                dxf: drawing.dxf,
                version: drawing.version,
                skipped: drawing
                    .skipped
                    .into_iter()
                    .map(|records| (records.record_type, records.count))
                    .collect(),
            },
            Err(error) => Self::Refused(error.into()),
        }
    }
}

impl From<DwgOutcome> for Result<ketchup_dwg::DwgDrawing, DwgRefusal> {
    fn from(outcome: DwgOutcome) -> Self {
        match outcome {
            DwgOutcome::Converted {
                dxf,
                version,
                skipped,
            } => Ok(ketchup_dwg::DwgDrawing {
                dxf,
                version,
                skipped: skipped
                    .into_iter()
                    .map(|(record_type, count)| ketchup_dwg::SkippedRecords { record_type, count })
                    .collect(),
            }),
            DwgOutcome::Refused(refusal) => Err(refusal),
        }
    }
}

/// Bytes as one CBOR byte string, not an array of numbers twice their size.
mod cbor_bytes {
    use serde::{Deserializer, Serializer, de};
    use std::fmt;

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        struct Bytes;
        impl de::Visitor<'_> for Bytes {
            type Value = Vec<u8>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a byte string")
            }
            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Vec<u8>, E> {
                Ok(bytes.to_vec())
            }
            fn visit_byte_buf<E: de::Error>(self, bytes: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(bytes)
            }
        }
        deserializer.deserialize_byte_buf(Bytes)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProgramInput {
    pub file_name: String,
    pub source: String,
    pub overrides: BTreeMap<String, f64>,
}

/// Every field of [`Evaluated`]; its JSON form leaves the reference model out,
/// the wire may not.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EvaluatedProgram {
    model: Box<ProgramModel>,
    reference_model: Option<Box<ProgramModel>>,
    log: Vec<String>,
    unused_overrides: Vec<String>,
    part_sources: BTreeMap<String, Vec<SourceLines>>,
}

impl From<Evaluated> for EvaluatedProgram {
    fn from(evaluated: Evaluated) -> Self {
        Self {
            model: Box::new(evaluated.model),
            reference_model: evaluated.reference_model.map(Box::new),
            log: evaluated.log,
            unused_overrides: evaluated.unused_overrides,
            part_sources: evaluated.part_sources,
        }
    }
}

impl From<EvaluatedProgram> for Evaluated {
    fn from(program: EvaluatedProgram) -> Self {
        Self {
            model: *program.model,
            reference_model: program.reference_model.map(|model| *model),
            log: program.log,
            unused_overrides: program.unused_overrides,
            part_sources: program.part_sources,
        }
    }
}

/// One evaluation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ProgramOutcome {
    Evaluated(EvaluatedProgram),
    Failed(ProgramError),
}

impl From<Result<Evaluated, ProgramError>> for ProgramOutcome {
    fn from(result: Result<Evaluated, ProgramError>) -> Self {
        match result {
            Ok(evaluated) => Self::Evaluated(evaluated.into()),
            Err(error) => Self::Failed(error),
        }
    }
}

impl From<ProgramOutcome> for Result<Evaluated, ProgramError> {
    fn from(outcome: ProgramOutcome) -> Self {
        match outcome {
            ProgramOutcome::Evaluated(program) => Ok(program.into()),
            ProgramOutcome::Failed(error) => Err(error),
        }
    }
}

/// [`ketchup_program::report`] of one evaluation. The report leaves out the
/// program lines of its parts; the caller has them in its evaluation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ReportOutcome {
    Reported(Box<Report>),
    /// The report did not fit its reply.
    Failed(ProgramError),
}

/// A model and the exact answers to refine its report with.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RefineInput {
    pub model: ProgramModel,
    pub exact: ExactShapes,
}

/// [`ketchup_program::refine`] of one model.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum RefineOutcome {
    Refined(Box<Refined>),
    /// The issues and relations did not fit their reply.
    Failed(ProgramError),
}

/// Describes a display mesh written to the requested file.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MeshReceipt {
    pub result_fingerprint: String,
    pub vertex_count: u32,
    pub triangle_count: u32,
    pub sha256: String,
}

/// Describes a volume mesh written to the requested file.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VolumeMeshReceipt {
    pub result_fingerprint: String,
    pub vertex_count: u32,
    pub tetrahedron_count: u32,
    pub boundary_triangle_count: u32,
    pub sha256: String,
    pub mesh_fingerprint: String,
}

/// Describes an exchange file written to the requested path.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExportReceipt {
    pub result_fingerprint: String,
    pub sha256: Option<String>,
}

/// Exact evidence of one imported STEP or IGES body.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkerImportPart {
    pub result_fingerprint: String,
    pub body_kind: String,
    pub topology_counts: [u32; 5],
    pub area_mm2: f64,
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub source_unit: String,
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairMeasure {
    pub common_volume_mm3: f64,
    pub common_thickness_mm: f64,
    pub common_contact_area_mm2: f64,
    /// `common_contact_area_mm2` by the left body's outward normal.
    pub contact_patches: Vec<ContactPatch>,
    pub distance_mm: f64,
}

/// A refused or failed request. Geometry codes keep the worker usable.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkerFailure {
    pub code: String,
    pub detail: Option<FailureDetail>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FailureDetail {
    pub diagnostic: String,
    pub operation: String,
    pub input_digest: String,
    pub backend: String,
}

impl WorkerFailure {
    pub fn code(code: &str) -> Self {
        Self {
            code: code.to_owned(),
            detail: None,
        }
    }

    pub fn invalid_request() -> Self {
        Self::code("invalid_request")
    }
}

/// A frame bounded only by its u32 length prefix, which `encode_frame` refuses to overflow.
pub const ANY_FRAME_BYTES: usize = usize::MAX;

/// Encodes one length-prefixed frame, refusing bodies above `max_bytes`.
pub fn encode_frame(message: &impl Serialize, max_bytes: usize) -> io::Result<Vec<u8>> {
    let mut frame = vec![0; 4];
    ciborium::into_writer(message, &mut frame).map_err(io::Error::other)?;
    let length = frame.len() - 4;
    if length > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("worker frame of {length} bytes exceeds the {max_bytes}-byte limit"),
        ));
    }
    let prefix =
        u32::try_from(length).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    frame[..4].copy_from_slice(&prefix.to_le_bytes());
    Ok(frame)
}

pub fn write_frame(
    writer: &mut impl Write,
    message: &impl Serialize,
    max_bytes: usize,
) -> io::Result<()> {
    writer.write_all(&encode_frame(message, max_bytes)?)?;
    writer.flush()
}

#[derive(Debug)]
pub enum Frame<T> {
    Message(T),
    /// The stream ended cleanly between frames.
    Closed,
    TooLarge,
    Malformed(String),
    Transport(String),
}

/// Reads one frame; the declared length is checked before the body is read.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read, max_bytes: usize) -> Frame<T> {
    let mut header = [0; 4];
    let mut filled = 0;
    while filled < header.len() {
        match reader.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Frame::Closed,
            Ok(0) => return Frame::Malformed("worker frame header was truncated".to_owned()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Frame::Transport(error.to_string()),
        }
    }
    let length = u32::from_le_bytes(header) as usize;
    if length > max_bytes {
        return Frame::TooLarge;
    }
    let mut body = Vec::with_capacity(length);
    match reader.take(length as u64).read_to_end(&mut body) {
        Ok(read) if read == length => {}
        Ok(_) => return Frame::Malformed("worker frame body was truncated".to_owned()),
        Err(error) => return Frame::Transport(error.to_string()),
    }
    let mut cursor = io::Cursor::new(body.as_slice());
    match ciborium::from_reader(&mut cursor) {
        Ok(message) if cursor.position() as usize == length => Frame::Message(message),
        Ok(_) => Frame::Malformed("worker frame has trailing bytes".to_owned()),
        Err(error) => Frame::Malformed(format!("worker frame does not decode: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(message: &WorkerReply) -> Frame<WorkerReply> {
        let bytes = encode_frame(message, MAX_REPLY_FRAME_BYTES).unwrap();
        read_frame(&mut io::Cursor::new(bytes), MAX_REPLY_FRAME_BYTES)
    }

    #[test]
    fn frames_round_trip_back_to_back_and_end_cleanly() {
        let replies = [
            WorkerReply::Hello {
                protocol: protocol_identity().to_owned(),
            },
            WorkerReply::Pair(PairMeasure {
                common_volume_mm3: 0.0,
                common_thickness_mm: 0.0,
                common_contact_area_mm2: -0.0,
                contact_patches: Vec::new(),
                distance_mm: f64::MIN_POSITIVE,
            }),
            WorkerReply::Failure(WorkerFailure::invalid_request()),
        ];
        let mut stream = Vec::new();
        for reply in &replies {
            write_frame(&mut stream, reply, MAX_REPLY_FRAME_BYTES).unwrap();
        }
        let mut reader = io::Cursor::new(stream);
        for reply in &replies {
            assert!(matches!(
                read_frame::<WorkerReply>(&mut reader, MAX_REPLY_FRAME_BYTES),
                Frame::Message(read) if read == *reply
            ));
        }
        assert!(matches!(
            read_frame::<WorkerReply>(&mut reader, MAX_REPLY_FRAME_BYTES),
            Frame::Closed
        ));
        // Floating-point values keep their exact bits, including the sign of zero.
        let Frame::Message(WorkerReply::Pair(measure)) = round_trip(&replies[1]) else {
            panic!("pair measure did not round-trip");
        };
        assert_eq!(
            measure.common_contact_area_mm2.to_bits(),
            (-0.0_f64).to_bits()
        );
    }

    #[test]
    fn oversized_declared_length_is_refused_before_the_body_is_read() {
        let mut bytes = ((MAX_REPLY_FRAME_BYTES + 1) as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(b"never read");
        let mut reader = io::Cursor::new(bytes);
        assert!(matches!(
            read_frame::<WorkerReply>(&mut reader, MAX_REPLY_FRAME_BYTES),
            Frame::TooLarge
        ));
        assert_eq!(reader.position(), 4);
        assert!(encode_frame(&vec![0_u8; 64], 16).is_err());
    }

    #[test]
    fn truncated_trailing_and_foreign_frames_are_malformed() {
        let valid = encode_frame(&WorkerReply::Done, MAX_REPLY_FRAME_BYTES).unwrap();
        let mut trailing = valid.clone();
        trailing.push(0);
        trailing[..4].copy_from_slice(&((valid.len() - 3) as u32).to_le_bytes());
        let mut foreign = 3_u32.to_le_bytes().to_vec();
        foreign.extend_from_slice(b"\xff\xff\xff");
        for bytes in [
            valid[..2].to_vec(),
            valid[..valid.len() - 1].to_vec(),
            trailing,
            foreign,
        ] {
            assert!(matches!(
                read_frame::<WorkerReply>(&mut io::Cursor::new(bytes), MAX_REPLY_FRAME_BYTES),
                Frame::Malformed(_)
            ));
        }
    }

    #[test]
    fn identity_covers_the_wire_type_sources() {
        assert_eq!(protocol_identity().len(), 64);
        assert_eq!(protocol_identity(), protocol_identity());
    }
}
