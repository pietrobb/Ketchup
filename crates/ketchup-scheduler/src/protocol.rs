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
use ketchup_core::exact_brep_graph::{ExactBRepGraph, MAX_EXACT_BREP_GRAPH_BYTES};
use ketchup_core::graph::sha256_hex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

/// A maximal graph plus bounded request metadata (CAM motions, source paths).
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
    pub left_transform: [f64; 16],
    pub right_transform: [f64; 16],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum WorkerReply {
    Hello { protocol: String },
    Done,
    Graph(WorkerExactBRepGraphResult),
    Mesh(MeshReceipt),
    VolumeMesh(VolumeMeshReceipt),
    Exported(ExportReceipt),
    Cam(CamSimulationWireEvidence),
    Xde(StepXdeWorkerEvidence),
    ImportPart(WorkerImportPart),
    PairLoaded { slot: usize },
    Pair(PairMeasure),
    Failure(WorkerFailure),
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairMeasure {
    pub common_volume_mm3: f64,
    pub common_contact_area_mm2: f64,
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
    frame[..4].copy_from_slice(&(length as u32).to_le_bytes());
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
                common_contact_area_mm2: -0.0,
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
