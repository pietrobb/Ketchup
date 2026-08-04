//! Narrow, exception-safe exact geometry boundary used by the A0 gate.

use std::fmt;

const BACKEND_FINGERPRINT: &str = env!("KETCHUP_OCCT_BUILD_FINGERPRINT");
const TOLERANCE_PROFILE: &str = "r0-v1:bbox=1e-6mm:volume_abs=1e-6mm3:volume_rel=1e-10";
const MIN_LENGTH_MM: f64 = 0.01;
const MAX_LENGTH_MM: f64 = 100_000.0;
const MAX_COORDINATE_MM: f64 = 1_000_000.0;

#[must_use]
pub const fn backend_fingerprint() -> &'static str {
    BACKEND_FINGERPRINT
}

#[must_use]
pub const fn tolerance_profile() -> &'static str {
    TOLERANCE_PROFILE
}

#[allow(dead_code, unsafe_code)]
#[cxx::bridge(namespace = "ketchup::exact")]
mod ffi {
    struct NativeTopologySummary {
        vertex_count: u32,
        edge_count: u32,
        face_count: u32,
        shell_count: u32,
        solid_count: u32,
        volume_mm3: f64,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
    }

    struct NativeFaceEvidence {
        ordinal: u32,
        surface_kind: String,
        area_mm2: f64,
        centroid_x: f64,
        centroid_y: f64,
        centroid_z: f64,
        normal_x: f64,
        normal_y: f64,
        normal_z: f64,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
        edge_count: u32,
    }

    struct NativeFaceEdgeEvidence {
        face_ordinal: u32,
        edge_ordinal: u32,
    }

    struct NativeEdgeFaceEvidence {
        edge_ordinal: u32,
        face_ordinal: u32,
    }

    struct NativeHistoryEvidence {
        semantic_role: String,
        relation: String,
        source_element_id: String,
        output_ordinal: u32,
        output_present: bool,
    }

    unsafe extern "C++" {
        include!("ketchup_exact.hxx");

        type NativeOperationResult;

        fn make_box_native(
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            size_x: f64,
            size_y: f64,
            size_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn extrude_rectangle_native(
            width: f64,
            depth: f64,
            height: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn cut_box_native(
            base: &NativeOperationResult,
            origin_x: f64,
            origin_y: f64,
            origin_z: f64,
            size_x: f64,
            size_y: f64,
            size_z: f64,
        ) -> UniquePtr<NativeOperationResult>;
        fn exception_probe_native() -> UniquePtr<NativeOperationResult>;
        fn import_step_native(path: &str) -> UniquePtr<NativeOperationResult>;

        fn status_code(self: &NativeOperationResult) -> u8;
        fn diagnostic(self: &NativeOperationResult) -> String;
        fn valid(self: &NativeOperationResult) -> bool;
        fn topology_summary(self: &NativeOperationResult) -> NativeTopologySummary;
        fn face_evidence(self: &NativeOperationResult) -> Vec<NativeFaceEvidence>;
        fn face_edge_evidence(self: &NativeOperationResult) -> Vec<NativeFaceEdgeEvidence>;
        fn edge_face_evidence(self: &NativeOperationResult) -> Vec<NativeEdgeFaceEvidence>;
        fn history_evidence(self: &NativeOperationResult) -> Vec<NativeHistoryEvidence>;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const ORIGIN: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxSpec {
    pub origin_mm: Point3,
    pub size_mm: Size3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleExtrudeSpec {
    pub width_mm: f64,
    pub depth_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CutMode {
    ThroughAll,
    BlindPlanar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryErrorCode {
    InvalidParameter,
    InvalidProfile,
    NonFiniteParameter,
    NoGeometricChange,
    DegenerateOperation,
    InvalidShape,
    BackendException,
    NullResult,
}

impl GeometryErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParameter => "invalid_parameter",
            Self::InvalidProfile => "invalid_profile",
            Self::NonFiniteParameter => "non_finite_parameter",
            Self::NoGeometricChange => "no_geometric_change",
            Self::DegenerateOperation => "degenerate_operation",
            Self::InvalidShape => "invalid_shape",
            Self::BackendException => "backend_exception",
            Self::NullResult => "null_result",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryError {
    pub code: GeometryErrorCode,
    pub diagnostic: String,
    pub operation: &'static str,
    pub input_digest: String,
    pub backend_fingerprint: &'static str,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.diagnostic)
    }
}

impl std::error::Error for GeometryError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds3 {
    pub min: Point3,
    pub max: Point3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FaceEvidence {
    pub ordinal: u32,
    pub surface_kind: String,
    pub area_mm2: f64,
    pub centroid_mm: Point3,
    pub normal: Point3,
    pub bounds_mm: Bounds3,
    pub edge_count: u32,
    pub edge_ordinals: Vec<u32>,
    pub geometric_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdgeEvidence {
    pub ordinal: u32,
    pub adjacent_face_ordinals: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyEvidence {
    pub vertex_count: u32,
    pub edge_count: u32,
    pub face_count: u32,
    pub shell_count: u32,
    pub solid_count: u32,
    pub volume_mm3: f64,
    pub bounds_mm: Bounds3,
    pub faces: Vec<FaceEvidence>,
    pub edges: Vec<EdgeEvidence>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEvidence {
    pub semantic_role: Option<String>,
    pub relation: String,
    pub source_element_id: String,
    pub output_face_ordinal: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryConfidence {
    Complete,
    Partial,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StabilityClass {
    Guaranteed,
    HistoryTracked,
    Heuristic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubshapeRef {
    pub document_id: String,
    pub producer_feature_id: String,
    pub semantic_role: String,
    pub source_element_id: String,
    pub expected_type: String,
    pub stability_class: StabilityClass,
    pub backend_fingerprint: String,
    pub lineage_digest: String,
    pub corroborating_geometry_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReferenceResolution {
    Resolved {
        face_ordinal: u32,
        migrated_backend: bool,
    },
    Ambiguous {
        candidate_ordinals: Vec<u32>,
    },
    Lost,
    QuarantinedMigration {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryDiagnostic {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToleranceReport {
    pub profile: &'static str,
    pub shape_valid: bool,
    pub accepted_exact_solid: bool,
}

pub struct ExactBody {
    native: cxx::UniquePtr<ffi::NativeOperationResult>,
    pub result_fingerprint: String,
    pub topology: TopologyEvidence,
}

impl fmt::Debug for ExactBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExactBody")
            .field("result_fingerprint", &self.result_fingerprint)
            .field("topology", &self.topology)
            .finish_non_exhaustive()
    }
}

pub struct ExactOpOutput {
    pub body: ExactBody,
    pub topology_history: Vec<HistoryEvidence>,
    pub tolerance_report: ToleranceReport,
    pub diagnostics: Vec<GeometryDiagnostic>,
    pub input_digest: String,
    pub backend_fingerprint: &'static str,
    pub history_confidence: HistoryConfidence,
}

impl fmt::Debug for ExactOpOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExactOpOutput")
            .field("body", &self.body)
            .field("topology_history", &self.topology_history)
            .field("tolerance_report", &self.tolerance_report)
            .field("diagnostics", &self.diagnostics)
            .field("input_digest", &self.input_digest)
            .field("backend_fingerprint", &self.backend_fingerprint)
            .field("history_confidence", &self.history_confidence)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ExactBackend;

impl ExactBackend {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn make_box(&self, spec: BoxSpec) -> Result<ExactOpOutput, GeometryError> {
        let input = box_input("box", spec);
        validate_box(spec, "box", &input)?;
        let native = ffi::make_box_native(
            spec.origin_mm.x,
            spec.origin_mm.y,
            spec.origin_mm.z,
            spec.size_mm.x,
            spec.size_mm.y,
            spec.size_mm.z,
        );
        collect_output(native, "box", &input, HistoryConfidence::None)
    }

    pub fn extrude_rectangle(
        &self,
        spec: RectangleExtrudeSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_rectangle:{:016x}:{:016x}:{:016x}",
            spec.width_mm.to_bits(),
            spec.depth_mm.to_bits(),
            spec.height_mm.to_bits()
        );
        validate_length(spec.width_mm, "width_mm", "extrude_rectangle", &input)?;
        validate_length(spec.depth_mm, "depth_mm", "extrude_rectangle", &input)?;
        validate_length(spec.height_mm, "height_mm", "extrude_rectangle", &input)?;
        let native = ffi::extrude_rectangle_native(spec.width_mm, spec.depth_mm, spec.height_mm);
        collect_output(
            native,
            "extrude_rectangle",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn exception_probe(&self) -> Result<ExactOpOutput, GeometryError> {
        collect_output(
            ffi::exception_probe_native(),
            "exception_probe",
            "intentional",
            HistoryConfidence::None,
        )
    }

    pub fn import_step(&self, path: &str) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_step:{path}");
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_step",
                &input,
                "STEP path must not be empty".to_owned(),
            ));
        }
        collect_output(
            ffi::import_step_native(path),
            "import_step",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn cut_box(
        &self,
        base: &ExactBody,
        tool: BoxSpec,
        mode: CutMode,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "cut_box:{}:{}:{:?}",
            base.result_fingerprint,
            box_input("tool", tool),
            mode
        );
        validate_box(tool, "cut_box", &input)?;
        classify_box_intersection(base.topology.bounds_mm, tool, &input)?;
        let native_base = base.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "cut_box",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native = ffi::cut_box_native(
            native_base,
            tool.origin_mm.x,
            tool.origin_mm.y,
            tool.origin_mm.z,
            tool.size_mm.x,
            tool.size_mm.y,
            tool.size_mm.z,
        );
        collect_output(native, "cut_box", &input, HistoryConfidence::Partial)
    }
}

fn collect_output(
    native: cxx::UniquePtr<ffi::NativeOperationResult>,
    operation: &'static str,
    input: &str,
    history_confidence: HistoryConfidence,
) -> Result<ExactOpOutput, GeometryError> {
    let input_digest = stable_digest(input);
    let native_ref = native.as_ref().ok_or_else(|| GeometryError {
        code: GeometryErrorCode::NullResult,
        diagnostic: "Native facade returned no result object".to_owned(),
        operation,
        input_digest: input_digest.clone(),
        backend_fingerprint: BACKEND_FINGERPRINT,
    })?;
    let status = native_ref.status_code();
    let diagnostic = native_ref.diagnostic();
    if status != 0 || !native_ref.valid() {
        return Err(GeometryError {
            code: native_status(status),
            diagnostic,
            operation,
            input_digest,
            backend_fingerprint: BACKEND_FINGERPRINT,
        });
    }

    let summary = native_ref.topology_summary();
    let face_edges = native_ref.face_edge_evidence();
    let edge_faces = native_ref.edge_face_evidence();
    let faces = native_ref
        .face_evidence()
        .into_iter()
        .map(|face| {
            let signature = format!(
                "{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                face.ordinal,
                face.surface_kind,
                face.area_mm2.to_bits(),
                face.centroid_x.to_bits(),
                face.centroid_y.to_bits(),
                face.centroid_z.to_bits(),
                face.normal_x.to_bits(),
                face.normal_y.to_bits(),
                face.edge_count
            );
            FaceEvidence {
                ordinal: face.ordinal,
                surface_kind: face.surface_kind,
                area_mm2: face.area_mm2,
                centroid_mm: Point3 {
                    x: face.centroid_x,
                    y: face.centroid_y,
                    z: face.centroid_z,
                },
                normal: Point3 {
                    x: face.normal_x,
                    y: face.normal_y,
                    z: face.normal_z,
                },
                bounds_mm: Bounds3 {
                    min: Point3 {
                        x: face.min_x,
                        y: face.min_y,
                        z: face.min_z,
                    },
                    max: Point3 {
                        x: face.max_x,
                        y: face.max_y,
                        z: face.max_z,
                    },
                },
                edge_count: face.edge_count,
                edge_ordinals: {
                    let mut ordinals = face_edges
                        .iter()
                        .filter(|entry| entry.face_ordinal == face.ordinal)
                        .map(|entry| entry.edge_ordinal)
                        .collect::<Vec<_>>();
                    ordinals.sort_unstable();
                    ordinals
                },
                geometric_fingerprint: stable_digest(&signature),
            }
        })
        .collect::<Vec<_>>();
    let topology = TopologyEvidence {
        vertex_count: summary.vertex_count,
        edge_count: summary.edge_count,
        face_count: summary.face_count,
        shell_count: summary.shell_count,
        solid_count: summary.solid_count,
        volume_mm3: summary.volume_mm3,
        bounds_mm: Bounds3 {
            min: Point3 {
                x: summary.min_x,
                y: summary.min_y,
                z: summary.min_z,
            },
            max: Point3 {
                x: summary.max_x,
                y: summary.max_y,
                z: summary.max_z,
            },
        },
        faces,
        edges: (0..summary.edge_count)
            .map(|ordinal| {
                let mut adjacent_face_ordinals = edge_faces
                    .iter()
                    .filter(|entry| entry.edge_ordinal == ordinal)
                    .map(|entry| entry.face_ordinal)
                    .collect::<Vec<_>>();
                adjacent_face_ordinals.sort_unstable();
                EdgeEvidence {
                    ordinal,
                    adjacent_face_ordinals,
                }
            })
            .collect(),
    };
    let result_signature = format!(
        "{}:{}:{}:{}:{}:{:016x}:{:?}",
        BACKEND_FINGERPRINT,
        operation,
        input_digest,
        topology.face_count,
        topology.solid_count,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm
    );
    let result_fingerprint = stable_digest(&result_signature);
    let history = native_ref
        .history_evidence()
        .into_iter()
        .map(|entry| HistoryEvidence {
            semantic_role: (!entry.semantic_role.is_empty()).then_some(entry.semantic_role),
            relation: entry.relation,
            source_element_id: entry.source_element_id,
            output_face_ordinal: entry.output_present.then_some(entry.output_ordinal),
        })
        .collect::<Vec<_>>();

    Ok(ExactOpOutput {
        body: ExactBody {
            native,
            result_fingerprint,
            topology,
        },
        topology_history: history,
        tolerance_report: ToleranceReport {
            profile: TOLERANCE_PROFILE,
            shape_valid: true,
            accepted_exact_solid: true,
        },
        diagnostics: vec![GeometryDiagnostic {
            code: "valid_exact_solid",
            message: diagnostic,
        }],
        input_digest,
        backend_fingerprint: BACKEND_FINGERPRINT,
        history_confidence,
    })
}

#[must_use]
pub fn has_complete_manifold_adjacency(topology: &TopologyEvidence) -> bool {
    if topology.faces.len() != topology.face_count as usize
        || topology.edges.len() != topology.edge_count as usize
        || topology
            .faces
            .iter()
            .enumerate()
            .any(|(ordinal, face)| face.ordinal != ordinal as u32)
        || topology
            .edges
            .iter()
            .enumerate()
            .any(|(ordinal, edge)| edge.ordinal != ordinal as u32)
    {
        return false;
    }

    topology.faces.iter().all(|face| {
        face.edge_count as usize == face.edge_ordinals.len()
            && !face.edge_ordinals.is_empty()
            && face.edge_ordinals.windows(2).all(|pair| pair[0] < pair[1])
            && face.edge_ordinals.iter().all(|edge_ordinal| {
                topology.edges.iter().any(|edge| {
                    edge.ordinal == *edge_ordinal
                        && edge.adjacent_face_ordinals.contains(&face.ordinal)
                })
            })
    }) && topology.edges.iter().all(|edge| {
        edge.adjacent_face_ordinals.len() == 2
            && edge.adjacent_face_ordinals[0] < edge.adjacent_face_ordinals[1]
            && edge.adjacent_face_ordinals.iter().all(|face_ordinal| {
                topology.faces.iter().any(|face| {
                    face.ordinal == *face_ordinal && face.edge_ordinals.contains(&edge.ordinal)
                })
            })
    })
}

pub fn capture_guaranteed_references(
    output: &ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_manifold_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_guaranteed_references",
            &output.input_digest,
            "Guaranteed output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let required = [
        ("extrusion.top", "profile.face"),
        ("extrusion.bottom", "profile.face"),
        ("extrusion.side(profile_edge=east)", "profile.edge.east"),
    ];
    let mut references = Vec::with_capacity(required.len());
    for (semantic_role, source_element_id) in required {
        let candidates = output
            .topology_history
            .iter()
            .filter(|entry| {
                entry.semantic_role.as_deref() == Some(semantic_role)
                    && entry.source_element_id == source_element_id
            })
            .filter_map(|entry| entry.output_face_ordinal)
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_guaranteed_references",
                &output.input_digest,
                format!(
                    "Guaranteed role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        }
        let ordinal = candidates[0];
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == ordinal && face.surface_kind == "plane")
            .ok_or_else(|| {
                parameter_error(
                    GeometryErrorCode::InvalidShape,
                    "capture_guaranteed_references",
                    &output.input_digest,
                    format!("Guaranteed role {semantic_role} is not a planar output face"),
                )
            })?;
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint: face.geometric_fingerprint.clone(),
        });
    }
    Ok(references)
}

pub fn capture_bounded_through_cut_references(
    output: &mut ExactOpOutput,
    document_id: &str,
    producer_feature_id: &str,
    base: RectangleExtrudeSpec,
    cut: BoxSpec,
) -> Result<Vec<SubshapeRef>, GeometryError> {
    if !has_complete_manifold_adjacency(&output.body.topology) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidShape,
            "capture_bounded_through_cut_references",
            &output.input_digest,
            "Through-cut output lacks complete reciprocal face/edge adjacency".to_owned(),
        ));
    }
    let cut_max_x = cut.origin_mm.x + cut.size_mm.x;
    let cut_max_y = cut.origin_mm.y + cut.size_mm.y;
    let roles = [
        ("extrusion.top", "profile.face", 2_usize, base.height_mm),
        ("extrusion.bottom", "profile.face", 2_usize, 0.0),
        (
            "extrusion.side(profile_edge=east)",
            "profile.edge.east",
            0_usize,
            base.width_mm,
        ),
        (
            "through_cut.wall.west",
            "cut_profile.edge.west",
            0_usize,
            cut.origin_mm.x,
        ),
        (
            "through_cut.wall.east",
            "cut_profile.edge.east",
            0_usize,
            cut_max_x,
        ),
        (
            "through_cut.wall.south",
            "cut_profile.edge.south",
            1_usize,
            cut.origin_mm.y,
        ),
        (
            "through_cut.wall.north",
            "cut_profile.edge.north",
            1_usize,
            cut_max_y,
        ),
    ];
    let mut references = Vec::with_capacity(roles.len());
    for (semantic_role, source_element_id, axis, coordinate) in roles {
        let candidates = output
            .body
            .topology
            .faces
            .iter()
            .filter(|face| face.surface_kind == "plane")
            .filter(|face| face_matches_cut_role(face, semantic_role, axis, coordinate, base, cut))
            .collect::<Vec<_>>();
        let [face] = candidates.as_slice() else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                "capture_bounded_through_cut_references",
                &output.input_digest,
                format!(
                    "Through-cut role {semantic_role} has {} candidates",
                    candidates.len()
                ),
            ));
        };
        let face_ordinal = face.ordinal;
        let corroborating_geometry_fingerprint = face.geometric_fingerprint.clone();
        if !output.topology_history.iter().any(|entry| {
            entry.semantic_role.as_deref() == Some(semantic_role)
                && entry.source_element_id == source_element_id
                && entry.output_face_ordinal == Some(face_ordinal)
        }) {
            output.topology_history.push(HistoryEvidence {
                semantic_role: Some(semantic_role.to_owned()),
                relation: "bounded_cut_classification".to_owned(),
                source_element_id: source_element_id.to_owned(),
                output_face_ordinal: Some(face_ordinal),
            });
        }
        let lineage = format!(
            "{document_id}:{producer_feature_id}:{semantic_role}:{source_element_id}:planar_face"
        );
        references.push(SubshapeRef {
            document_id: document_id.to_owned(),
            producer_feature_id: producer_feature_id.to_owned(),
            semantic_role: semantic_role.to_owned(),
            source_element_id: source_element_id.to_owned(),
            expected_type: "planar_face".to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: output.backend_fingerprint.to_owned(),
            lineage_digest: stable_digest(&lineage),
            corroborating_geometry_fingerprint,
        });
    }
    Ok(references)
}

fn face_matches_cut_role(
    face: &FaceEvidence,
    semantic_role: &str,
    axis: usize,
    coordinate: f64,
    base: RectangleExtrudeSpec,
    cut: BoxSpec,
) -> bool {
    let mins = [
        face.bounds_mm.min.x,
        face.bounds_mm.min.y,
        face.bounds_mm.min.z,
    ];
    let maxs = [
        face.bounds_mm.max.x,
        face.bounds_mm.max.y,
        face.bounds_mm.max.z,
    ];
    if (mins[axis] - coordinate).abs() > 1.0e-6 || (maxs[axis] - coordinate).abs() > 1.0e-6 {
        return false;
    }
    let cut_max_x = cut.origin_mm.x + cut.size_mm.x;
    let cut_max_y = cut.origin_mm.y + cut.size_mm.y;
    match semantic_role {
        "extrusion.top" | "extrusion.bottom" => {
            (mins[0]).abs() <= 1.0e-6
                && (mins[1]).abs() <= 1.0e-6
                && (maxs[0] - base.width_mm).abs() <= 1.0e-6
                && (maxs[1] - base.depth_mm).abs() <= 1.0e-6
        }
        "extrusion.side(profile_edge=east)" => {
            mins[1].abs() <= 1.0e-6
                && mins[2].abs() <= 1.0e-6
                && (maxs[1] - base.depth_mm).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        "through_cut.wall.west" | "through_cut.wall.east" => {
            (mins[1] - cut.origin_mm.y).abs() <= 1.0e-6
                && mins[2].abs() <= 1.0e-6
                && (maxs[1] - cut_max_y).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        "through_cut.wall.south" | "through_cut.wall.north" => {
            (mins[0] - cut.origin_mm.x).abs() <= 1.0e-6
                && mins[2].abs() <= 1.0e-6
                && (maxs[0] - cut_max_x).abs() <= 1.0e-6
                && (maxs[2] - base.height_mm).abs() <= 1.0e-6
        }
        _ => false,
    }
}

#[must_use]
pub fn resolve_subshape_reference(
    reference: &SubshapeRef,
    output: &ExactOpOutput,
) -> ReferenceResolution {
    let migrated_backend = reference.backend_fingerprint != output.backend_fingerprint;
    let expected_lineage = stable_digest(&format!(
        "{}:{}:{}:{}:{}",
        reference.document_id,
        reference.producer_feature_id,
        reference.semantic_role,
        reference.source_element_id,
        reference.expected_type
    ));
    if reference.document_id.is_empty()
        || reference.producer_feature_id.is_empty()
        || reference.lineage_digest != expected_lineage
    {
        return if migrated_backend {
            ReferenceResolution::QuarantinedMigration {
                reason: "Reference provenance or lineage digest is invalid".to_owned(),
            }
        } else {
            ReferenceResolution::Lost
        };
    }
    let mut candidates = output
        .topology_history
        .iter()
        .filter(|entry| {
            entry.semantic_role.as_deref() == Some(reference.semantic_role.as_str())
                && entry.source_element_id == reference.source_element_id
        })
        .filter_map(|entry| entry.output_face_ordinal)
        .filter(|ordinal| {
            output.body.topology.faces.iter().any(|face| {
                face.ordinal == *ordinal
                    && (reference.expected_type != "planar_face" || face.surface_kind == "plane")
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    match candidates.as_slice() {
        [face_ordinal] => ReferenceResolution::Resolved {
            face_ordinal: *face_ordinal,
            migrated_backend,
        },
        [] if migrated_backend => ReferenceResolution::QuarantinedMigration {
            reason: format!(
                "No lineage match for {} after backend migration",
                reference.semantic_role
            ),
        },
        [] => ReferenceResolution::Lost,
        _ => ReferenceResolution::Ambiguous {
            candidate_ordinals: candidates,
        },
    }
}

pub fn validate_closed_planar_profile(points: &[Point3]) -> Result<(), GeometryError> {
    let input = format!("profile:{points:?}");
    if points.len() != 4
        || points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite())
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            "validate_profile",
            &input,
            "A0 supports exactly four finite planar profile vertices".to_owned(),
        ));
    }
    let z = points[0].z;
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left.x * right.y - right.x * left.y)
        .sum::<f64>();
    if points.iter().any(|point| (point.z - z).abs() > 1.0e-9)
        || twice_area.abs() <= 1.0e-12
        || segments_intersect(points[0], points[1], points[2], points[3])
        || segments_intersect(points[1], points[2], points[3], points[0])
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            "validate_profile",
            &input,
            "Profile is non-planar, degenerate, or self-intersecting".to_owned(),
        ));
    }
    Ok(())
}

fn segments_intersect(a: Point3, b: Point3, c: Point3, d: Point3) -> bool {
    fn orientation(a: Point3, b: Point3, c: Point3) -> f64 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }
    let first = orientation(a, b, c);
    let second = orientation(a, b, d);
    let third = orientation(c, d, a);
    let fourth = orientation(c, d, b);
    first * second < 0.0 && third * fourth < 0.0
}

fn native_status(status: u8) -> GeometryErrorCode {
    match status {
        1 => GeometryErrorCode::InvalidParameter,
        2 => GeometryErrorCode::NonFiniteParameter,
        3 => GeometryErrorCode::NoGeometricChange,
        4 => GeometryErrorCode::DegenerateOperation,
        5 => GeometryErrorCode::InvalidShape,
        6 => GeometryErrorCode::BackendException,
        _ => GeometryErrorCode::NullResult,
    }
}

fn validate_box(spec: BoxSpec, operation: &'static str, input: &str) -> Result<(), GeometryError> {
    for (value, name) in [
        (spec.origin_mm.x, "origin_x"),
        (spec.origin_mm.y, "origin_y"),
        (spec.origin_mm.z, "origin_z"),
    ] {
        validate_coordinate(value, name, operation, input)?;
    }
    for (value, name) in [
        (spec.size_mm.x, "size_x"),
        (spec.size_mm.y, "size_y"),
        (spec.size_mm.z, "size_z"),
    ] {
        validate_length(value, name, operation, input)?;
    }
    for (origin, size, name) in [
        (spec.origin_mm.x, spec.size_mm.x, "max_x"),
        (spec.origin_mm.y, spec.size_mm.y, "max_y"),
        (spec.origin_mm.z, spec.size_mm.z, "max_z"),
    ] {
        validate_coordinate(origin + size, name, operation, input)?;
    }
    Ok(())
}

fn validate_length(
    value: f64,
    name: &str,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    if !value.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            operation,
            input,
            format!("{name} must be finite"),
        ));
    }
    if !(MIN_LENGTH_MM..=MAX_LENGTH_MM).contains(&value) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            format!("{name} must be within {MIN_LENGTH_MM}..={MAX_LENGTH_MM} mm"),
        ));
    }
    Ok(())
}

fn validate_coordinate(
    value: f64,
    name: &str,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    if !value.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            operation,
            input,
            format!("{name} must be finite"),
        ));
    }
    if value.abs() > MAX_COORDINATE_MM {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            format!("{name} exceeds the local coordinate envelope"),
        ));
    }
    Ok(())
}

fn classify_box_intersection(
    base: Bounds3,
    tool: BoxSpec,
    input: &str,
) -> Result<(), GeometryError> {
    let tool_max = Point3 {
        x: tool.origin_mm.x + tool.size_mm.x,
        y: tool.origin_mm.y + tool.size_mm.y,
        z: tool.origin_mm.z + tool.size_mm.z,
    };
    let overlaps = [
        base.max.x.min(tool_max.x) - base.min.x.max(tool.origin_mm.x),
        base.max.y.min(tool_max.y) - base.min.y.max(tool.origin_mm.y),
        base.max.z.min(tool_max.z) - base.min.z.max(tool.origin_mm.z),
    ];
    const INTERSECTION_TOLERANCE_MM: f64 = 1.0e-6;
    if overlaps
        .iter()
        .any(|overlap| *overlap < -INTERSECTION_TOLERANCE_MM)
    {
        return Err(parameter_error(
            GeometryErrorCode::NoGeometricChange,
            "cut_box",
            input,
            "Cut tool does not intersect the exact body's bounds".to_owned(),
        ));
    }
    if overlaps
        .iter()
        .any(|overlap| overlap.abs() <= INTERSECTION_TOLERANCE_MM)
    {
        return Err(parameter_error(
            GeometryErrorCode::DegenerateOperation,
            "cut_box",
            input,
            "Cut tool only touches the exact body's bounds".to_owned(),
        ));
    }
    Ok(())
}

fn parameter_error(
    code: GeometryErrorCode,
    operation: &'static str,
    input: &str,
    diagnostic: String,
) -> GeometryError {
    GeometryError {
        code,
        diagnostic,
        operation,
        input_digest: stable_digest(input),
        backend_fingerprint: BACKEND_FINGERPRINT,
    }
}

fn box_input(label: &str, spec: BoxSpec) -> String {
    format!(
        "{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
        label,
        spec.origin_mm.x.to_bits(),
        spec.origin_mm.y.to_bits(),
        spec.origin_mm.z.to_bits(),
        spec.size_mm.x.to_bits(),
        spec.size_mm.y.to_bits(),
        spec.size_mm.z.to_bits()
    )
}

fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayToken {
    pub seed: u64,
    pub case_index: u32,
}

impl fmt::Display for ReplayToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "r0-v1:{}:{}", self.seed, self.case_index)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GeneratedOperation {
    Extrude(RectangleExtrudeSpec),
    Cut {
        base: BoxSpec,
        tool: BoxSpec,
        mode: CutMode,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedCase {
    pub replay: ReplayToken,
    pub operation: GeneratedOperation,
}

#[derive(Clone, Copy, Debug)]
pub struct StructuredGenerator {
    seed: u64,
}

impl StructuredGenerator {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    #[must_use]
    pub fn case(&self, case_index: u32) -> GeneratedCase {
        let mut random = SplitMix64::new(self.seed);
        for _ in 0..case_index {
            let _ = random.next();
        }
        let selector = random.next() % 100;
        let operation = if selector < 50 {
            GeneratedOperation::Extrude(RectangleExtrudeSpec {
                width_mm: generated_length(&mut random, 10_000, 100_000_000),
                depth_mm: generated_length(&mut random, 10_000, 100_000_000),
                height_mm: generated_length(&mut random, 10_000, 100_000_000),
            })
        } else {
            let base_size = Size3 {
                x: generated_length(&mut random, 30_000, 100_000_000),
                y: generated_length(&mut random, 30_000, 100_000_000),
                z: generated_length(&mut random, 30_000, 99_998_000),
            };
            let tool_x = bounded_tool_length(&mut random, base_size.x);
            let tool_y = bounded_tool_length(&mut random, base_size.y);
            let margin_x = (base_size.x - tool_x) / 2.0;
            let margin_y = (base_size.y - tool_y) / 2.0;
            let mode = if selector < 80 {
                CutMode::ThroughAll
            } else {
                CutMode::BlindPlanar
            };
            let (origin_z, size_z) = match mode {
                CutMode::ThroughAll => (-1.0, base_size.z + 2.0),
                CutMode::BlindPlanar => (base_size.z / 2.0, base_size.z / 2.0 + 1.0),
            };
            GeneratedOperation::Cut {
                base: BoxSpec {
                    origin_mm: Point3::ORIGIN,
                    size_mm: base_size,
                },
                tool: BoxSpec {
                    origin_mm: Point3 {
                        x: margin_x,
                        y: margin_y,
                        z: origin_z,
                    },
                    size_mm: Size3 {
                        x: tool_x,
                        y: tool_y,
                        z: size_z,
                    },
                },
                mode,
            }
        };
        GeneratedCase {
            replay: ReplayToken {
                seed: self.seed,
                case_index,
            },
            operation,
        }
    }
}

impl ExactBackend {
    pub fn execute_generated(&self, case: &GeneratedCase) -> Result<ExactOpOutput, GeometryError> {
        match case.operation {
            GeneratedOperation::Extrude(spec) => self.extrude_rectangle(spec),
            GeneratedOperation::Cut { base, tool, mode } => {
                let base = self.make_box(base)?;
                self.cut_box(&base.body, tool, mode)
            }
        }
    }
}

#[must_use]
pub fn minimize_replayable_failure<F>(case: &GeneratedCase, mut still_fails: F) -> GeneratedCase
where
    F: FnMut(&GeneratedCase) -> bool,
{
    if !still_fails(case) {
        return case.clone();
    }
    let mut current = case.clone();
    loop {
        let candidate = shrink_case(&current);
        if candidate == current || !still_fails(&candidate) {
            return current;
        }
        current = candidate;
    }
}

fn shrink_case(case: &GeneratedCase) -> GeneratedCase {
    let operation = match case.operation {
        GeneratedOperation::Extrude(spec) => GeneratedOperation::Extrude(RectangleExtrudeSpec {
            width_mm: shrink_length(spec.width_mm),
            depth_mm: shrink_length(spec.depth_mm),
            height_mm: shrink_length(spec.height_mm),
        }),
        GeneratedOperation::Cut {
            base,
            tool: _,
            mode,
        } => {
            let size = Size3 {
                x: shrink_length(base.size_mm.x),
                y: shrink_length(base.size_mm.y),
                z: shrink_length(base.size_mm.z),
            };
            let tool_x = (size.x / 2.0).max(MIN_LENGTH_MM);
            let tool_y = (size.y / 2.0).max(MIN_LENGTH_MM);
            let (origin_z, size_z) = match mode {
                CutMode::ThroughAll => (-MIN_LENGTH_MM, size.z + 2.0 * MIN_LENGTH_MM),
                CutMode::BlindPlanar => (size.z / 2.0, size.z / 2.0 + MIN_LENGTH_MM),
            };
            GeneratedOperation::Cut {
                base: BoxSpec {
                    origin_mm: Point3::ORIGIN,
                    size_mm: size,
                },
                tool: BoxSpec {
                    origin_mm: Point3 {
                        x: (size.x - tool_x) / 2.0,
                        y: (size.y - tool_y) / 2.0,
                        z: origin_z,
                    },
                    size_mm: Size3 {
                        x: tool_x,
                        y: tool_y,
                        z: size_z,
                    },
                },
                mode,
            }
        }
    };
    GeneratedCase {
        replay: case.replay,
        operation,
    }
}

fn shrink_length(value: f64) -> f64 {
    ((value + MIN_LENGTH_MM) / 2.0).max(MIN_LENGTH_MM)
}

fn generated_length(random: &mut SplitMix64, minimum_um: u64, maximum_um: u64) -> f64 {
    let span = maximum_um - minimum_um + 1;
    (minimum_um + random.next() % span) as f64 / 1000.0
}

fn bounded_tool_length(random: &mut SplitMix64, base_mm: f64) -> f64 {
    let base_um = (base_mm * 1000.0).round() as u64;
    let maximum_um = base_um.saturating_sub(2_000).max(10_000);
    generated_length(random, 10_000, maximum_um)
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1.0e-6,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn fixed_extrusion_is_valid_and_carries_guaranteed_history() {
        let output = ExactBackend::new()
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: 100.0,
                depth_mm: 60.0,
                height_mm: 20.0,
            })
            .expect("fixed extrusion must succeed");

        assert_eq!(output.body.topology.solid_count, 1);
        assert_eq!(output.body.topology.face_count, 6);
        assert_close(output.body.topology.volume_mm3, 120_000.0);
        assert_eq!(output.history_confidence, HistoryConfidence::Complete);
        for role in [
            "extrusion.top",
            "extrusion.bottom",
            "extrusion.side(profile_edge=east)",
        ] {
            assert!(output.topology_history.iter().any(|entry| {
                entry.semantic_role.as_deref() == Some(role) && entry.output_face_ordinal.is_some()
            }));
        }
        let references = capture_guaranteed_references(&output, "unit-document", "extrusion-001")
            .expect("Guaranteed references must be captured from backend history");
        for (role, expected_axis_coordinate, axis) in [
            ("extrusion.top", 20.0, 'z'),
            ("extrusion.bottom", 0.0, 'z'),
            ("extrusion.side(profile_edge=east)", 100.0, 'x'),
        ] {
            let reference = references
                .iter()
                .find(|reference| reference.semantic_role == role)
                .expect("Guaranteed reference must exist");
            let ReferenceResolution::Resolved { face_ordinal, .. } =
                resolve_subshape_reference(reference, &output)
            else {
                panic!("Guaranteed reference must resolve uniquely");
            };
            let face = output
                .body
                .topology
                .faces
                .iter()
                .find(|face| face.ordinal == face_ordinal)
                .expect("Guaranteed face ordinal must exist");
            assert_eq!(face.surface_kind, "plane");
            let coordinate = if axis == 'x' {
                face.centroid_mm.x
            } else {
                face.centroid_mm.z
            };
            assert_close(coordinate, expected_axis_coordinate);
        }
        assert!(output.input_digest.starts_with("fnv1a64:"));
        assert!(output.body.result_fingerprint.starts_with("fnv1a64:"));
    }

    #[test]
    fn fixed_through_cut_is_valid_and_captures_history() {
        let backend = ExactBackend::new();
        let base = backend
            .make_box(BoxSpec {
                origin_mm: Point3::ORIGIN,
                size_mm: Size3 {
                    x: 40.0,
                    y: 30.0,
                    z: 10.0,
                },
            })
            .expect("base must succeed");
        let output = backend
            .cut_box(
                &base.body,
                BoxSpec {
                    origin_mm: Point3 {
                        x: 10.0,
                        y: 10.0,
                        z: -5.0,
                    },
                    size_mm: Size3 {
                        x: 20.0,
                        y: 10.0,
                        z: 20.0,
                    },
                },
                CutMode::ThroughAll,
            )
            .expect("fixed cut must succeed");

        assert_eq!(output.body.topology.solid_count, 1);
        assert_close(output.body.topology.volume_mm3, 10_000.0);
        assert_eq!(output.history_confidence, HistoryConfidence::Partial);
        assert!(!output.topology_history.is_empty());
    }

    #[test]
    fn invalid_and_degenerate_inputs_have_stable_typed_errors() {
        let backend = ExactBackend::new();
        let error = backend
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: 0.0,
                depth_mm: 10.0,
                height_mm: 10.0,
            })
            .expect_err("zero width must be rejected");
        assert_eq!(error.code, GeometryErrorCode::InvalidParameter);

        let error = backend
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: f64::NAN,
                depth_mm: 10.0,
                height_mm: 10.0,
            })
            .expect_err("non-finite width must be rejected");
        assert_eq!(error.code, GeometryErrorCode::NonFiniteParameter);

        let base = backend
            .make_box(BoxSpec {
                origin_mm: Point3::ORIGIN,
                size_mm: Size3 {
                    x: 10.0,
                    y: 10.0,
                    z: 10.0,
                },
            })
            .expect("base must succeed");
        let error = backend
            .cut_box(
                &base.body,
                BoxSpec {
                    origin_mm: Point3 {
                        x: 10.0,
                        y: 2.0,
                        z: 2.0,
                    },
                    size_mm: Size3 {
                        x: 2.0,
                        y: 2.0,
                        z: 2.0,
                    },
                },
                CutMode::BlindPlanar,
            )
            .expect_err("touch-only cut must be rejected");
        assert_eq!(error.code, GeometryErrorCode::DegenerateOperation);
    }

    #[test]
    fn native_occt_exception_is_contained_by_the_facade() {
        let error = collect_output(
            ffi::exception_probe_native(),
            "exception_probe",
            "intentional",
            HistoryConfidence::None,
        )
        .expect_err("probe must become a typed boundary error");
        assert_eq!(error.code, GeometryErrorCode::BackendException);
        assert!(
            error
                .diagnostic
                .contains("intentional A0 exception-boundary probe")
        );
    }

    #[test]
    fn structured_cases_replay_and_execute() {
        let backend = ExactBackend::new();
        for seed in [1, 7, 19] {
            let generator = StructuredGenerator::new(seed);
            for index in 0..5 {
                let first = generator.case(index);
                let replayed =
                    StructuredGenerator::new(first.replay.seed).case(first.replay.case_index);
                assert_eq!(first, replayed);
                let output = backend
                    .execute_generated(&first)
                    .unwrap_or_else(|error| panic!("{} failed: {error}", first.replay));
                assert!(output.tolerance_report.accepted_exact_solid);
            }
        }
    }

    #[test]
    fn minimizer_preserves_replay_token_and_structure() {
        let case = GeneratedCase {
            replay: ReplayToken {
                seed: 43,
                case_index: 9,
            },
            operation: GeneratedOperation::Extrude(RectangleExtrudeSpec {
                width_mm: 100.0,
                depth_mm: 60.0,
                height_mm: 20.0,
            }),
        };
        let minimized = minimize_replayable_failure(&case, |candidate| match candidate.operation {
            GeneratedOperation::Extrude(spec) => spec.width_mm > 20.0,
            GeneratedOperation::Cut { .. } => false,
        });
        assert_eq!(minimized.replay, case.replay);
        let GeneratedOperation::Extrude(spec) = minimized.operation else {
            panic!("minimizer changed operation structure");
        };
        assert!(spec.width_mm > 20.0);
        assert!(spec.width_mm < 100.0);
    }
}
