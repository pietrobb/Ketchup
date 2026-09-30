use super::*;

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
    pub axis_origin_mm: Option<Point3>,
    pub axis_direction: Option<Point3>,
    pub bounds_mm: Bounds3,
    pub edge_count: u32,
    pub edge_ordinals: Vec<u32>,
    pub geometric_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdgeEvidence {
    pub ordinal: u32,
    pub curve_kind: String,
    pub length_mm: f64,
    pub centroid_mm: Point3,
    pub bounds_mm: Bounds3,
    pub closed: bool,
    pub circle_radius_mm: Option<f64>,
    pub axis_origin_mm: Option<Point3>,
    pub axis_direction: Option<Point3>,
    pub adjacent_face_ordinals: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyEvidence {
    pub vertex_count: u32,
    pub edge_count: u32,
    pub wire_count: u32,
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
    pub output_edge_ordinal: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HalfLapParticipant {
    A,
    B,
}

impl HalfLapParticipant {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HalfLapFaceRole {
    Contact,
    WestWall,
    EastWall,
}

impl HalfLapFaceRole {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::WestWall => "wall.west",
            Self::EastWall => "wall.east",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HalfLapNotchSpec {
    pub joint_id: u64,
    pub participant: HalfLapParticipant,
    pub removed: BoxSpec,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HalfLapFaceEvidence {
    pub joint_id: u64,
    pub participant: HalfLapParticipant,
    pub role: HalfLapFaceRole,
    pub face_ordinal: u32,
    pub lineage_digest: String,
    pub geometric_fingerprint: String,
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
    ResolvedEdge {
        edge_ordinal: u32,
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

/// One triangle of a derived display mesh, tagged with the ordinal of the
/// exact face it was tessellated from so shading and picking can stay per-face.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactMeshTriangle {
    pub vertex_indices: [u32; 3],
    pub face_ordinal: u32,
}

/// A derived, non-canonical display mesh of an exact body.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactTessellation {
    pub vertices_mm: Vec<[f64; 3]>,
    pub triangles: Vec<ExactMeshTriangle>,
}

pub const EXACT_VOLUME_MESH_SCHEMA: &str = "ketchup.exact-volume-mesh.v1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactVolumeMeshOptions {
    pub surface_deflection_mm: f64,
    pub angular_deflection_rad: f64,
    pub max_tetrahedra: u32,
    pub max_relative_volume_error: f64,
    pub min_tetrahedron_quality: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactVolumeMeshTetrahedron {
    pub vertex_indices: [u32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactVolumeMesh {
    pub schema: &'static str,
    pub source_result_fingerprint: String,
    pub request_digest: String,
    pub mesh_fingerprint: String,
    pub vertices_mm: Vec<[f64; 3]>,
    pub tetrahedra: Vec<ExactVolumeMeshTetrahedron>,
    pub boundary_triangles: Vec<ExactMeshTriangle>,
    pub exact_volume_mm3: f64,
    pub tetrahedral_volume_mm3: f64,
    pub relative_volume_error: f64,
    pub minimum_signed_volume_mm3: f64,
    pub minimum_quality: f64,
    pub maximum_edge_ratio: f64,
}

/// Solid-set relation computed from OCCT BRep common volume and distance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactPairRelation {
    /// Strictly positive common solid volume, including full containment.
    Penetrating,
    /// No common volume and distance at most the requested contact tolerance.
    Touching,
    /// No common volume and distance greater than the contact tolerance.
    Separated,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactPairQueryResult {
    pub relation: ExactPairRelation,
    pub common_volume_mm3: f64,
    /// Area of zero-volume common faces; positive only for face contact.
    pub common_contact_area_mm2: f64,
    /// Minimum solid-set distance; zero for penetrating/contained bodies.
    pub distance_mm: f64,
}

pub struct ExactBody {
    pub(super) native: cxx::UniquePtr<ffi::NativeOperationResult>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactBodyBooleanOperation {
    Cut,
    Union,
    Intersect,
    Split,
}
