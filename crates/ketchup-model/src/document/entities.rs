use super::*;
use ketchup_geometry::linalg::Affine3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnitSystem {
    Millimetres,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform {
    pub(super) matrix: [f64; 16],
}

impl Transform {
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn from_matrix(matrix: [f64; 16]) -> Result<Self, CanonicalError> {
        if matrix.iter().all(|value| value.is_finite())
            && matrix[12] == 0.0
            && matrix[13] == 0.0
            && matrix[14] == 0.0
            && matrix[15] == 1.0
        {
            Ok(Self { matrix })
        } else {
            Err(CanonicalError::InvalidTransform)
        }
    }

    pub fn from_translation(x_mm: f64, y_mm: f64, z_mm: f64) -> Result<Self, CanonicalError> {
        let mut matrix = Self::identity().matrix;
        matrix[3] = x_mm;
        matrix[7] = y_mm;
        matrix[11] = z_mm;
        Self::from_matrix(matrix)
    }

    #[must_use]
    pub const fn matrix(&self) -> &[f64; 16] {
        &self.matrix
    }

    #[must_use]
    pub fn compose(self, local: Self) -> Self {
        Self {
            matrix: self.affine().compose(local.affine()).to_row_major(),
        }
    }

    #[must_use]
    pub fn affine(self) -> Affine3 {
        Affine3::from_row_major(self.matrix)
    }

    #[must_use]
    pub fn transform_point(self, point: [f64; 3]) -> [f64; 3] {
        self.affine().transform_point(point.into()).into()
    }

    #[must_use]
    pub fn transform_vector(self, vector: [f64; 3]) -> [f64; 3] {
        self.affine().transform_vector(vector.into()).into()
    }

    pub fn from_affine(affine: Affine3) -> Result<Self, CanonicalError> {
        Self::from_matrix(affine.to_row_major())
    }

    /// The inverse of any non-singular affine transform.
    #[must_use]
    pub fn inverse(self) -> Option<Self> {
        Self::from_affine(self.affine().invert().ok()?).ok()
    }

    /// The inverse of a rotation plus translation; `None` when the linear
    /// part is not orthonormal within [`ROUNDING`].
    #[must_use]
    pub fn rigid_inverse(self) -> Option<Self> {
        let affine = self.affine();
        if !affine.linear.is_orthonormal(ROUNDING) {
            return None;
        }
        let linear = affine.linear.transpose();
        Self::from_affine(Affine3 {
            linear,
            translation: -linear.mul_vec(affine.translation),
        })
        .ok()
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EdgeFinishKind {
    Fillet,
    Chamfer,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ChamferMode {
    Symmetric,
    TwoDistance { second_distance: Dimension },
    DistanceAngle { angle_degrees: f64 },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChamferEdgeSide {
    pub edge: TopologicalElementRef,
    pub side_face: TopologicalElementRef,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ProfileFaceReference {
    Start,
    End,
    Segment { entity_id: u64, source_name: String },
    NamedResult(String),
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProfileEdgeReference {
    pub first: ProfileFaceReference,
    pub second: ProfileFaceReference,
}

/// A face picked by a feature: a recorded topological element of the target
/// body, or a face named by the profile that produced it.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FaceRef {
    Topological(Box<TopologicalElementRef>),
    Named(ProfileFaceReference),
}

/// An edge picked by a feature, referenced the same two ways as [`FaceRef`].
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EdgeRef {
    Topological(Box<TopologicalElementRef>),
    Named(ProfileEdgeReference),
}

impl From<TopologicalElementRef> for FaceRef {
    fn from(reference: TopologicalElementRef) -> Self {
        Self::Topological(Box::new(reference))
    }
}

impl From<TopologicalElementRef> for EdgeRef {
    fn from(reference: TopologicalElementRef) -> Self {
        Self::Topological(Box::new(reference))
    }
}

impl ProfileFaceReference {
    /// A segment face names its sketch entity and source; a named result is
    /// non-empty.
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Start | Self::End => true,
            Self::Segment {
                entity_id,
                source_name,
            } => *entity_id != 0 && !source_name.is_empty(),
            Self::NamedResult(name) => !name.is_empty(),
        }
    }
}

impl FaceRef {
    /// The recorded references of `faces`, or `None` when any face is named.
    pub fn all_topological(faces: &[Self]) -> Option<Vec<TopologicalElementRef>> {
        faces
            .iter()
            .map(|face| face.topological().cloned())
            .collect()
    }

    /// The named references of `faces`, or `None` when any face is recorded.
    pub fn all_named(faces: &[Self]) -> Option<Vec<ProfileFaceReference>> {
        faces.iter().map(|face| face.named().cloned()).collect()
    }

    pub fn topological(&self) -> Option<&TopologicalElementRef> {
        match self {
            Self::Topological(reference) => Some(reference.as_ref()),
            Self::Named(_) => None,
        }
    }

    pub fn named(&self) -> Option<&ProfileFaceReference> {
        match self {
            Self::Named(reference) => Some(reference),
            Self::Topological(_) => None,
        }
    }
}

impl EdgeRef {
    /// The recorded references of `edges`, or `None` when any edge is named.
    pub fn all_topological(edges: &[Self]) -> Option<Vec<TopologicalElementRef>> {
        edges
            .iter()
            .map(|edge| edge.topological().cloned())
            .collect()
    }

    /// The named references of `edges`, or `None` when any edge is recorded.
    pub fn all_named(edges: &[Self]) -> Option<Vec<ProfileEdgeReference>> {
        edges.iter().map(|edge| edge.named().cloned()).collect()
    }

    pub fn topological(&self) -> Option<&TopologicalElementRef> {
        match self {
            Self::Topological(reference) => Some(reference.as_ref()),
            Self::Named(_) => None,
        }
    }

    pub fn named(&self) -> Option<&ProfileEdgeReference> {
        match self {
            Self::Named(reference) => Some(reference),
            Self::Topological(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ShellDirection {
    #[default]
    Inward,
    Outward,
    Symmetric,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StableEdgeRole(pub(super) String);

impl StableEdgeRole {
    pub fn new(role: impl Into<String>) -> Result<Self, CanonicalError> {
        let role = role.into();
        validate_stable_subshape_role(&role)?;
        Ok(Self(role))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub const MAX_PARAMETER_PATH_BYTES: usize = 256;

#[derive(
    Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct ParameterPath(pub(super) String);

impl ParameterPath {
    pub fn new(path: impl Into<String>) -> Result<Self, ParameterPathError> {
        let path = path.into();
        if path.is_empty() {
            return Err(ParameterPathError::Empty);
        }
        if path.len() > MAX_PARAMETER_PATH_BYTES {
            return Err(ParameterPathError::TooLong);
        }
        if path.split('.').any(|segment| {
            segment.is_empty()
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }) {
            return Err(ParameterPathError::InvalidSegment);
        }
        Ok(Self(path))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterPathError {
    Empty,
    TooLong,
    InvalidSegment,
}

impl fmt::Display for ParameterPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "parameter path is empty",
            Self::TooLong => "parameter path exceeds the resource limit",
            Self::InvalidSegment => "parameter path contains an invalid segment",
        })
    }
}

impl std::error::Error for ParameterPathError {}

#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum ParameterValueType {
    Length,
    Angle,
    Scalar,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterDescriptor {
    pub(super) path: ParameterPath,
    pub(super) value_type: ParameterValueType,
}

impl ParameterDescriptor {
    pub fn new(
        path: impl Into<String>,
        value_type: ParameterValueType,
    ) -> Result<Self, ParameterPathError> {
        Ok(Self {
            path: ParameterPath::new(path)?,
            value_type,
        })
    }

    #[must_use]
    pub fn path(&self) -> &ParameterPath {
        &self.path
    }

    #[must_use]
    pub const fn value_type(&self) -> ParameterValueType {
        self.value_type
    }
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct FeatureParameterTarget {
    pub feature_id: FeatureId,
    pub path: ParameterPath,
    pub value_type: ParameterValueType,
}

impl FeatureParameterTarget {
    pub fn new(
        feature_id: FeatureId,
        path: impl Into<String>,
        value_type: ParameterValueType,
    ) -> Result<Self, ParameterPathError> {
        Ok(Self {
            feature_id,
            path: ParameterPath::new(path)?,
            value_type,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FeatureParameterBinding {
    pub target: FeatureParameterTarget,
    pub derived_from: DerivedIdentity,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvaluatorParameterEdit {
    SetDimension {
        id: NodeId,
        dimension: Dimension,
    },
    SetExpression {
        id: NodeId,
        expression: String,
    },
    SetRuleOutputs {
        id: NodeId,
        outputs: Vec<RuleOutput>,
    },
}

impl EvaluatorParameterEdit {
    #[must_use]
    pub const fn affected_node(&self) -> NodeId {
        match self {
            Self::SetDimension { id, .. }
            | Self::SetExpression { id, .. }
            | Self::SetRuleOutputs { id, .. } => *id,
        }
    }

    pub(super) fn into_command(self) -> CanonicalCommand {
        match self {
            Self::SetDimension { id, dimension } => {
                CanonicalCommand::SetEvaluatorDimension { id, dimension }
            }
            Self::SetExpression { id, expression } => {
                CanonicalCommand::SetNodeExpression { id, expression }
            }
            Self::SetRuleOutputs { id, outputs } => {
                CanonicalCommand::SetRuleOutputs { id, outputs }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub enum FeatureParameterRecomputeScope {
    All,
    AffectedBy(BTreeSet<NodeId>),
}

impl FeatureParameterRecomputeScope {
    #[must_use]
    pub fn affected_by(node: NodeId) -> Self {
        Self::AffectedBy(BTreeSet::from([node]))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FeatureParameterProvenance {
    pub identity: EvaluationIdentity,
    pub input_digest: String,
    pub result_digest: String,
    pub applied_value_bits: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureParameterStaleReason {
    NeverComputed,
    EvaluatorChanged,
    SchemaChanged,
    ToleranceChanged,
    BackendChanged,
    InputChanged,
    ResultChanged,
    AppliedValueChanged,
    EvaluationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureParameterFreshness {
    Current,
    Stale(FeatureParameterStaleReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureParameterFreshnessAudit {
    pub target: FeatureParameterTarget,
    pub freshness: FeatureParameterFreshness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BooleanOperation {
    Cut,
    Union,
    Intersect,
    Split,
}

pub const MESH_BODY_SCHEMA_V1: &str = "ketchup.mesh-body.v1";
pub const IMPORTED_EXACT_BODY_SCHEMA_V1: &str = "ketchup.imported-exact-body.v1";
pub const IMPORTED_EXACT_BODY_SCHEMA_V2: &str = "ketchup.imported-exact-body.v2";
pub const IMPORTED_EXACT_BODY_SCHEMA_V3: &str = "ketchup.imported-exact-body.v3";

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ExactReferenceConversionConsequence {
    Lost,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExactToMeshConversion {
    pub source_document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub source_definition_id: DefinitionId,
    pub source_feature_id: FeatureId,
    pub source_result_fingerprint: String,
    pub source_evaluator: String,
    pub source_backend: String,
    pub source_tolerance: String,
    pub tessellation_tolerance: String,
    pub destination_definition_id: DefinitionId,
    pub destination_feature_id: FeatureId,
    pub unsupported_semantics: Vec<String>,
    pub exact_reference_consequence: ExactReferenceConversionConsequence,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MeshAuthority {
    Authored { provenance: String },
    ExactConversion(ExactToMeshConversion),
    ImportedStl { import_id: ImportId },
    ImportedSketchupScene { import_id: ImportId },
    ImportedGlb { import_id: ImportId },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImportedExactBodySpec {
    pub schema: String,
    pub import_id: ImportId,
    pub source_sha256: [u8; 32],
    pub source_byte_len: u64,
    pub source_part_index: Option<u32>,
    pub result_fingerprint: String,
    pub body_kind: BodyKind,
    pub solid_count: u32,
    pub topology_counts: Option<[u32; 5]>,
    pub area_mm2: f64,
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeshBodySpec {
    pub schema: String,
    pub vertices_mm: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub authority: MeshAuthority,
}

impl MeshBodySpec {
    #[must_use]
    pub fn exact_conversion_loss_report(&self) -> Option<String> {
        let MeshAuthority::ExactConversion(conversion) = &self.authority else {
            return None;
        };
        Some(format!(
            "authority=canonical mesh body\nconversion=exact-to-mesh\nsource_document_id={}\nsource_revision={}\nsource_digest={}\nsource_definition_id={}\nsource_feature_id={}\nsource_result_fingerprint={}\nsource_evaluator={}\nsource_backend={}\nsource_tolerance={}\ntessellation_tolerance={}\ndestination_definition_id={}\ndestination_feature_id={}\neditability_loss=canonical exact features, rules, and dimensions are not preserved\ntopology_loss=exact topology and analytic surfaces are not preserved\ntolerance_loss=geometry is approximated by the accepted tessellation\nexact_reference_consequence=Lost\nunsupported_semantics={}\n",
            conversion.source_document_id.0,
            conversion.source_revision,
            conversion.source_digest,
            conversion.source_definition_id.0,
            conversion.source_feature_id.0,
            conversion.source_result_fingerprint,
            conversion.source_evaluator,
            conversion.source_backend,
            conversion.source_tolerance,
            conversion.tessellation_tolerance,
            conversion.destination_definition_id.0,
            conversion.destination_feature_id.0,
            conversion.unsupported_semantics.join(",")
        ))
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ProfileSegment {
    Line {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
    },
    CircularArc {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
    CubicBezier {
        start_mm: [f64; 2],
        control_1_mm: [f64; 2],
        control_2_mm: [f64; 2],
        end_mm: [f64; 2],
    },
    /// A smooth curve through `points_mm`, from the first point to the last. A spline
    /// that ends where it starts is smooth through that point too.
    Spline { points_mm: Vec<[f64; 2]> },
}

/// Fewest distinct points a spline passes through: it is cubic.
pub const SPLINE_MIN_POINTS: usize = 4;

/// Stands for the end of a spline without points; it equals no point, so the
/// profile holding it is invalid.
pub(super) const MISSING_POINT: [f64; 2] = [f64::NAN; 2];

impl ProfileSegment {
    #[must_use]
    pub fn start_mm(&self) -> [f64; 2] {
        match self {
            Self::Line { start_mm, .. }
            | Self::CircularArc { start_mm, .. }
            | Self::CubicBezier { start_mm, .. } => *start_mm,
            Self::Spline { points_mm } => points_mm.first().copied().unwrap_or(MISSING_POINT),
        }
    }

    #[must_use]
    pub fn end_mm(&self) -> [f64; 2] {
        match self {
            Self::Line { end_mm, .. }
            | Self::CircularArc { end_mm, .. }
            | Self::CubicBezier { end_mm, .. } => *end_mm,
            Self::Spline { points_mm } => points_mm.last().copied().unwrap_or(MISSING_POINT),
        }
    }

    pub(super) fn start_mut(&mut self) -> Option<&mut [f64; 2]> {
        match self {
            Self::Line { start_mm, .. }
            | Self::CircularArc { start_mm, .. }
            | Self::CubicBezier { start_mm, .. } => Some(start_mm),
            Self::Spline { points_mm } => points_mm.first_mut(),
        }
    }

    pub(super) fn end_mut(&mut self) -> Option<&mut [f64; 2]> {
        match self {
            Self::Line { end_mm, .. }
            | Self::CircularArc { end_mm, .. }
            | Self::CubicBezier { end_mm, .. } => Some(end_mm),
            Self::Spline { points_mm } => points_mm.last_mut(),
        }
    }

    /// Every point that places the segment, in order.
    #[must_use]
    pub fn defining_points_mm(&self) -> Vec<[f64; 2]> {
        match self {
            Self::Line { start_mm, end_mm } => vec![*start_mm, *end_mm],
            Self::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => vec![*start_mm, *end_mm, *center_mm],
            Self::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => vec![*start_mm, *control_1_mm, *control_2_mm, *end_mm],
            Self::Spline { points_mm } => points_mm.clone(),
        }
    }

    pub(super) fn defining_points_mut(&mut self) -> Vec<&mut [f64; 2]> {
        match self {
            Self::Line { start_mm, end_mm } => vec![start_mm, end_mm],
            Self::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => vec![start_mm, end_mm, center_mm],
            Self::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => vec![start_mm, control_1_mm, control_2_mm, end_mm],
            Self::Spline { points_mm } => points_mm.iter_mut().collect(),
        }
    }
}

/// The closing lines of a polygon through `points_mm`.
#[must_use]
pub fn polygon_segments(points_mm: &[[f64; 2]]) -> Vec<ProfileSegment> {
    points_mm
        .iter()
        .zip(points_mm.iter().cycle().skip(1))
        .map(|(start, end)| ProfileSegment::Line {
            start_mm: *start,
            end_mm: *end,
        })
        .collect()
}

/// The corners of a regular polygon with `sides` corners on a circle of `radius_mm`
/// around the origin, the first corner `first_corner_radians` from +x, counter-clockwise.
#[must_use]
pub fn regular_polygon_points(
    sides: usize,
    radius_mm: f64,
    first_corner_radians: f64,
) -> Vec<[f64; 2]> {
    let step = std::f64::consts::TAU / sides as f64;
    (0..sides)
        .map(|corner| {
            let (sin, cos) = (first_corner_radians + step * corner as f64).sin_cos();
            [radius_mm * cos, radius_mm * sin]
        })
        .collect()
}

/// An ellipse with half-axes `radius_x_mm` (turned `rotation_radians` from +x) and
/// `radius_y_mm` around `center_mm`: four quarter cubics counter-clockwise from the
/// +x end, each within 0.03 % of the true ellipse.
#[must_use]
pub fn ellipse_segments(
    center_mm: [f64; 2],
    radius_x_mm: f64,
    radius_y_mm: f64,
    rotation_radians: f64,
) -> Vec<ProfileSegment> {
    // 4/3 (sqrt(2) - 1) puts the quarter cubic through the arc's midpoint.
    let kappa = 4.0 * (std::f64::consts::SQRT_2 - 1.0) / 3.0;
    let (sin, cos) = rotation_radians.sin_cos();
    let place = |[x, y]: [f64; 2]| {
        [
            center_mm[0] + cos * x - sin * y,
            center_mm[1] + sin * x + cos * y,
        ]
    };
    let (rx, ry) = (radius_x_mm, radius_y_mm);
    let ends = [[rx, 0.0], [0.0, ry], [-rx, 0.0], [0.0, -ry]];
    let tangents = [[0.0, ry], [-rx, 0.0], [0.0, -ry], [rx, 0.0]];
    (0..4)
        .map(|quarter| {
            let next = (quarter + 1) % 4;
            let (start, end) = (ends[quarter], ends[next]);
            ProfileSegment::CubicBezier {
                start_mm: place(start),
                control_1_mm: place([
                    start[0] + kappa * tangents[quarter][0],
                    start[1] + kappa * tangents[quarter][1],
                ]),
                control_2_mm: place([
                    end[0] - kappa * tangents[next][0],
                    end[1] - kappa * tangents[next][1],
                ]),
                end_mm: place(end),
            }
        })
        .collect()
}

/// The corners of a closed chain of straight lines, or `None` when a segment is curved
/// or the chain is not connected end to start.
#[must_use]
pub fn polygon_points(segments: &[ProfileSegment]) -> Option<Vec<[f64; 2]>> {
    segments
        .iter()
        .zip(segments.iter().cycle().skip(1))
        .map(|(segment, next)| match segment {
            ProfileSegment::Line { start_mm, end_mm } if *end_mm == next.start_mm() => {
                Some(*start_mm)
            }
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SpatialPathSegment {
    Line {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    },
    CircularArc {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        center_mm: [f64; 3],
        normal: [f64; 3],
        clockwise: bool,
    },
    CubicBezier {
        start_mm: [f64; 3],
        control_1_mm: [f64; 3],
        control_2_mm: [f64; 3],
        end_mm: [f64; 3],
    },
}

impl SpatialPathSegment {
    #[must_use]
    pub const fn start_mm(&self) -> [f64; 3] {
        match self {
            Self::Line { start_mm, .. }
            | Self::CircularArc { start_mm, .. }
            | Self::CubicBezier { start_mm, .. } => *start_mm,
        }
    }

    #[must_use]
    pub const fn end_mm(&self) -> [f64; 3] {
        match self {
            Self::Line { end_mm, .. }
            | Self::CircularArc { end_mm, .. }
            | Self::CubicBezier { end_mm, .. } => *end_mm,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LoftContinuity {
    Position,
    Tangent,
    Curvature,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoftSection {
    pub profile: FeatureId,
    pub elevation_mm: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FilletRadiusStation {
    pub position: f64,
    pub radius: Dimension,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SurfaceBodySpec {
    Planar {
        profile: FeatureId,
    },
    Loft {
        sections: Vec<LoftSection>,
        guide: Option<FeatureId>,
        continuity: LoftContinuity,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BodyKind {
    Solid,
    Surface,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WeldmentMemberSpec {
    pub profile: FeatureId,
    pub path: FeatureId,
    pub orientation_degrees: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WeldmentJointPolicy {
    Butt,
    Miter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WeldmentJointPrimary {
    First,
    Second,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WeldmentJointSpec {
    pub first_member: FeatureId,
    pub second_member: FeatureId,
    pub policy: WeldmentJointPolicy,
    pub primary: WeldmentJointPrimary,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FeatureKind {
    Workplane(WorkplaneSpec),
    Sketch(SketchSpec),
    Profile {
        segments: Vec<ProfileSegment>,
        closed: bool,
    },
    SpatialPath {
        segments: Vec<SpatialPathSegment>,
    },
    ConstructionPoint {
        position_mm: [f64; 3],
    },
    ConstructionAxis {
        origin_mm: [f64; 3],
        direction: [f64; 3],
    },
    ConstructionPlane {
        origin_mm: [f64; 3],
        normal: [f64; 3],
        x_direction: [f64; 3],
    },
    Pad(PadSpec),
    Revolve {
        profile: FeatureId,
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    },
    Shell {
        target: FeatureId,
        /// Faces left open; none makes a closed hollow body.
        removed_faces: Vec<FaceRef>,
        thickness: Dimension,
        direction: ShellDirection,
    },
    EdgeFinish {
        target: FeatureId,
        edges: Vec<EdgeRef>,
        kind: EdgeFinishKind,
        amount: Dimension,
        fillet_radius_stations: Vec<FilletRadiusStation>,
        chamfer_mode: ChamferMode,
        chamfer_edge_sides: Vec<ChamferEdgeSide>,
    },
    FaceOffset {
        target: FeatureId,
        face: FaceRef,
        distance: Dimension,
    },
    Boolean {
        operation: BooleanOperation,
        target: FeatureId,
        tool: FeatureId,
    },
    PlanarOffset {
        profile: FeatureId,
        distance: Dimension,
    },
    Sweep {
        profile: FeatureId,
        path: FeatureId,
        /// Along a spatial path, keeps the profile's v on this direction
        /// and its u on `tangent × up` (fixed binormal), so a profile swept
        /// along a helix about `up` stays in the axial section. None
        /// carries the profile square to the path without twist.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        up: Option<[f64; 3]>,
    },
    WeldmentMember(WeldmentMemberSpec),
    WeldmentJoint(WeldmentJointSpec),
    SurfaceBody(SurfaceBodySpec),
    SurfaceTrim {
        target: FeatureId,
        cutter: FeatureId,
    },
    SurfaceExtend {
        target: FeatureId,
        distance: Dimension,
    },
    SurfaceKnit {
        surfaces: Vec<FeatureId>,
        tolerance: Dimension,
        make_solid: bool,
    },
    SurfaceThicken {
        target: FeatureId,
        thickness: Dimension,
        direction: ShellDirection,
    },
    Loft {
        sections: Vec<LoftSection>,
        guide: Option<FeatureId>,
        continuity: LoftContinuity,
    },
    SheetMetal(SheetMetalSpec),
    ImportedExactBody(ImportedExactBodySpec),
    RigidTransform {
        target: FeatureId,
        transform: Transform,
    },
    MeshBody(MeshBodySpec),
}

impl FeatureKind {
    /// The target body and the recorded faces or edges a feature picks on it;
    /// `None` for features that pick no topology.
    pub fn topological_picks(&self) -> Option<(FeatureId, Vec<&TopologicalElementRef>)> {
        match self {
            Self::Shell {
                target,
                removed_faces,
                ..
            } => Some((
                *target,
                removed_faces
                    .iter()
                    .filter_map(FaceRef::topological)
                    .collect(),
            )),
            Self::EdgeFinish { target, edges, .. } => Some((
                *target,
                edges.iter().filter_map(EdgeRef::topological).collect(),
            )),
            Self::FaceOffset { target, face, .. } => {
                Some((*target, face.topological().into_iter().collect()))
            }
            _ => None,
        }
    }

    /// A closed profile of straight lines through `points_mm` in order.
    #[must_use]
    pub fn polygon(points_mm: &[[f64; 2]]) -> Self {
        Self::Profile {
            segments: polygon_segments(points_mm),
            closed: true,
        }
    }

    /// The corners of a closed profile made only of straight lines, in boundary order.
    #[must_use]
    pub fn polygon_points(&self) -> Option<Vec<[f64; 2]>> {
        match self {
            Self::Profile {
                segments,
                closed: true,
            } => polygon_points(segments),
            _ => None,
        }
    }

    /// Whether this is a rectangle with sides along the axes, starting at its minimum
    /// corner and running counter-clockwise: the profile whose prism is its own box.
    #[must_use]
    pub fn is_axis_aligned_rectangle(&self) -> bool {
        self.polygon_points()
            .is_some_and(|points| is_axis_aligned_rectangle(&points))
    }

    /// A closed profile that is one smooth curve through `points_mm` and back to the
    /// first point.
    #[must_use]
    pub fn closed_spline(points_mm: &[[f64; 2]]) -> Self {
        Self::Profile {
            segments: vec![ProfileSegment::Spline {
                points_mm: points_mm.iter().chain(points_mm.first()).copied().collect(),
            }],
            closed: true,
        }
    }

    /// The points of a closed profile that is one spline, without the repeated closing
    /// point.
    #[must_use]
    pub fn closed_spline_points(&self) -> Option<&[[f64; 2]]> {
        match self {
            Self::Profile {
                segments,
                closed: true,
            } => match segments.as_slice() {
                [ProfileSegment::Spline { points_mm }] => {
                    points_mm.split_last().map(|(_, rest)| rest)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// A new body swept `height` along the normal of a profile feature.
    #[must_use]
    pub fn extrusion(profile: FeatureId, height: Dimension) -> Self {
        Self::Pad(PadSpec {
            profile: PadProfile::Feature(profile),
            direction: FeatureDirection::AlongNormal,
            extent: FeatureExtent::Blind(height),
            operation: PadOperation::NewBody,
        })
    }

    /// `profile` swept along `path` without a fixed `up`.
    #[must_use]
    pub fn sweep(profile: FeatureId, path: FeatureId) -> Self {
        Self::Sweep {
            profile,
            path,
            up: None,
        }
    }

    /// A blind cut `depth` into `target`, opened on the target face the
    /// profile normal points out of.
    #[must_use]
    pub fn pocket(target: FeatureId, profile: FeatureId, depth: Dimension) -> Self {
        Self::Pad(PadSpec {
            profile: PadProfile::Feature(profile),
            direction: FeatureDirection::AlongNormal,
            extent: FeatureExtent::Blind(depth),
            operation: PadOperation::Cut {
                target,
                start: CutStart::TargetFace,
            },
        })
    }

    /// A cut through the whole `target` along the profile normal.
    #[must_use]
    pub fn through_cut(target: FeatureId, profile: FeatureId) -> Self {
        Self::Pad(PadSpec {
            profile: PadProfile::Feature(profile),
            direction: FeatureDirection::AlongNormal,
            extent: FeatureExtent::ThroughAll,
            operation: PadOperation::Cut {
                target,
                start: CutStart::ProfilePlane,
            },
        })
    }

    #[must_use]
    pub fn parameter_descriptors(&self) -> Vec<ParameterDescriptor> {
        let mut descriptors = Vec::new();
        match self {
            Self::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Free,
                ..
            }) => {
                for axis in ["x", "y", "z"] {
                    push_parameter_descriptor(
                        &mut descriptors,
                        format!("frame.origin.{axis}"),
                        ParameterValueType::Length,
                    );
                }
            }
            Self::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Offset { .. },
                ..
            }) => push_parameter_descriptor(
                &mut descriptors,
                "support.offset.distance",
                ParameterValueType::Length,
            ),
            Self::Sketch(spec) => {
                if spec.rectangle_bounds().is_some() {
                    for path in ["bounds.width", "bounds.height"] {
                        push_parameter_descriptor(
                            &mut descriptors,
                            path,
                            ParameterValueType::Length,
                        );
                    }
                }
                for entity in &spec.entities {
                    if spec.is_projected_entity(entity.id()) {
                        continue;
                    }
                    let id = entity.id().0;
                    match entity {
                        SketchEntity::Line { .. } => {
                            for point in ["start", "end"] {
                                for axis in ["x", "y"] {
                                    push_parameter_descriptor(
                                        &mut descriptors,
                                        format!("entities.{id}.{point}.{axis}"),
                                        ParameterValueType::Length,
                                    );
                                }
                            }
                        }
                        SketchEntity::Arc { .. } => {
                            for point in ["start", "end", "center"] {
                                for axis in ["x", "y"] {
                                    push_parameter_descriptor(
                                        &mut descriptors,
                                        format!("entities.{id}.{point}.{axis}"),
                                        ParameterValueType::Length,
                                    );
                                }
                            }
                        }
                        SketchEntity::Circle { .. } => {
                            for axis in ["x", "y"] {
                                push_parameter_descriptor(
                                    &mut descriptors,
                                    format!("entities.{id}.center.{axis}"),
                                    ParameterValueType::Length,
                                );
                            }
                            push_parameter_descriptor(
                                &mut descriptors,
                                format!("entities.{id}.radius"),
                                ParameterValueType::Length,
                            );
                        }
                        SketchEntity::CubicBezier { .. } => {
                            for point in ["start", "control_1", "control_2", "end"] {
                                for axis in ["x", "y"] {
                                    push_parameter_descriptor(
                                        &mut descriptors,
                                        format!("entities.{id}.{point}.{axis}"),
                                        ParameterValueType::Length,
                                    );
                                }
                            }
                        }
                    }
                }
                for constraint in &spec.constraints {
                    let id = constraint.id.0;
                    match constraint.kind {
                        SketchConstraintKind::Distance { .. }
                        | SketchConstraintKind::Radius { .. } => push_parameter_descriptor(
                            &mut descriptors,
                            format!("constraints.{id}.value"),
                            ParameterValueType::Length,
                        ),
                        SketchConstraintKind::Angle { .. } => push_parameter_descriptor(
                            &mut descriptors,
                            format!("constraints.{id}.angle"),
                            ParameterValueType::Angle,
                        ),
                        SketchConstraintKind::FixedPoint { .. } => {
                            for axis in ["x", "y"] {
                                push_parameter_descriptor(
                                    &mut descriptors,
                                    format!("constraints.{id}.position.{axis}"),
                                    ParameterValueType::Length,
                                );
                            }
                        }
                        SketchConstraintKind::Horizontal { .. }
                        | SketchConstraintKind::Vertical { .. }
                        | SketchConstraintKind::Coincident { .. }
                        | SketchConstraintKind::Parallel { .. }
                        | SketchConstraintKind::Perpendicular { .. }
                        | SketchConstraintKind::Tangent { .. }
                        | SketchConstraintKind::Equal { .. }
                        | SketchConstraintKind::Symmetric { .. }
                        | SketchConstraintKind::Concentric { .. }
                        | SketchConstraintKind::Collinear { .. }
                        | SketchConstraintKind::Midpoint { .. }
                        | SketchConstraintKind::PointOnCurve { .. }
                        | SketchConstraintKind::Projection { .. }
                        | SketchConstraintKind::Construction { .. } => {}
                    }
                }
            }
            Self::Profile { segments, closed } => {
                if self
                    .polygon_points()
                    .is_some_and(|points| is_axis_aligned_rectangle(&points))
                {
                    push_parameter_descriptor(
                        &mut descriptors,
                        "bounds.width",
                        ParameterValueType::Length,
                    );
                    push_parameter_descriptor(
                        &mut descriptors,
                        "bounds.height",
                        ParameterValueType::Length,
                    );
                }
                for (index, segment) in segments.iter().enumerate() {
                    // A vertex shared with the next segment is that segment's start.
                    let next = segments.get(index + 1).or(closed
                        .then(|| &segments[0])
                        .filter(|_| index + 1 == segments.len()));
                    let end_is_shared =
                        next.is_some_and(|next| next.start_mm() == segment.end_mm());
                    let ends = if end_is_shared {
                        &["start"][..]
                    } else {
                        &["start", "end"][..]
                    };
                    for point in ends {
                        for axis in ["x", "y"] {
                            push_parameter_descriptor(
                                &mut descriptors,
                                format!("segments.{index}.{point}.{axis}"),
                                ParameterValueType::Length,
                            );
                        }
                    }
                    let inner_points = match segment {
                        ProfileSegment::CircularArc { .. } => vec!["center".to_owned()],
                        ProfileSegment::Spline { points_mm } => (1..points_mm.len().max(1) - 1)
                            .map(|point| format!("points.{point}"))
                            .collect(),
                        ProfileSegment::Line { .. } | ProfileSegment::CubicBezier { .. } => {
                            Vec::new()
                        }
                    };
                    for point in inner_points {
                        for axis in ["x", "y"] {
                            push_parameter_descriptor(
                                &mut descriptors,
                                format!("segments.{index}.{point}.{axis}"),
                                ParameterValueType::Length,
                            );
                        }
                    }
                }
            }
            Self::Pad(spec) => describe_feature_extent(&mut descriptors, "extent", &spec.extent),
            Self::Revolve { .. } => {
                for point in ["axis_start", "axis_end"] {
                    for axis in ["x", "y"] {
                        push_parameter_descriptor(
                            &mut descriptors,
                            format!("{point}.{axis}"),
                            ParameterValueType::Length,
                        );
                    }
                }
                push_parameter_descriptor(&mut descriptors, "angle", ParameterValueType::Angle);
            }
            Self::Shell { .. } | Self::SurfaceThicken { .. } => {
                push_parameter_descriptor(&mut descriptors, "thickness", ParameterValueType::Length)
            }
            Self::EdgeFinish {
                fillet_radius_stations,
                chamfer_mode,
                ..
            } => {
                push_parameter_descriptor(&mut descriptors, "amount", ParameterValueType::Length);
                match chamfer_mode {
                    ChamferMode::TwoDistance { .. } => push_parameter_descriptor(
                        &mut descriptors,
                        "chamfer_second_distance",
                        ParameterValueType::Length,
                    ),
                    ChamferMode::DistanceAngle { .. } => push_parameter_descriptor(
                        &mut descriptors,
                        "chamfer_angle",
                        ParameterValueType::Angle,
                    ),
                    ChamferMode::Symmetric => {}
                }
                for (index, _) in fillet_radius_stations.iter().enumerate() {
                    push_parameter_descriptor(
                        &mut descriptors,
                        format!("fillet_radius_stations.{index}.radius"),
                        ParameterValueType::Length,
                    );
                }
            }
            Self::FaceOffset { .. } | Self::PlanarOffset { .. } | Self::SurfaceExtend { .. } => {
                push_parameter_descriptor(&mut descriptors, "distance", ParameterValueType::Length);
            }
            Self::SurfaceKnit { .. } => {
                push_parameter_descriptor(&mut descriptors, "tolerance", ParameterValueType::Length)
            }
            Self::WeldmentMember(_) => push_parameter_descriptor(
                &mut descriptors,
                "orientation",
                ParameterValueType::Angle,
            ),
            Self::WeldmentJoint(_) => {}
            Self::SheetMetal(spec) => {
                push_parameter_descriptor(
                    &mut descriptors,
                    "thickness",
                    ParameterValueType::Length,
                );
                push_parameter_descriptor(&mut descriptors, "k_factor", ParameterValueType::Scalar);
                for index in 0..spec.base_mm.len() {
                    for axis in ["x", "y"] {
                        push_parameter_descriptor(
                            &mut descriptors,
                            format!("base.{index}.{axis}"),
                            ParameterValueType::Length,
                        );
                    }
                }
                for index in 0..spec.bends.len() {
                    for (field, value_type) in [
                        ("length", ParameterValueType::Length),
                        ("angle", ParameterValueType::Angle),
                        ("inner_radius", ParameterValueType::Length),
                    ] {
                        push_parameter_descriptor(
                            &mut descriptors,
                            format!("bends.{index}.{field}"),
                            value_type,
                        );
                    }
                }
            }
            Self::Loft { sections, .. }
            | Self::SurfaceBody(SurfaceBodySpec::Loft { sections, .. }) => {
                for (index, _) in sections.iter().enumerate() {
                    push_parameter_descriptor(
                        &mut descriptors,
                        format!("sections.{index}.elevation"),
                        ParameterValueType::Length,
                    );
                }
            }
            Self::Boolean { .. }
            | Self::Sweep { .. }
            | Self::SpatialPath { .. }
            | Self::ConstructionPoint { .. }
            | Self::ConstructionAxis { .. }
            | Self::ConstructionPlane { .. }
            | Self::ImportedExactBody(_)
            | Self::SurfaceBody(SurfaceBodySpec::Planar { .. })
            | Self::SurfaceTrim { .. }
            | Self::RigidTransform { .. }
            | Self::MeshBody(_)
            | Self::Workplane(_) => {}
        }
        descriptors
    }

    #[must_use]
    pub fn parameter_value(&self, path: &ParameterPath) -> Option<f64> {
        feature_kind_parameter_value(self, path.as_str())
    }

    #[must_use]
    pub fn dependencies(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Workplane(spec) => match &spec.support {
                WorkplaneSupport::Free | WorkplaneSupport::Principal(_) => BTreeSet::new(),
                WorkplaneSupport::Offset { base, .. } => [*base].into_iter().collect(),
                WorkplaneSupport::ConstructionPlane { feature } => [*feature].into_iter().collect(),
                WorkplaneSupport::PlanarFace { reference, .. } => {
                    [reference.profile_feature_id, reference.producer_feature_id]
                        .into_iter()
                        .collect()
                }
            },
            Self::Sketch(spec) => std::iter::once(spec.workplane)
                .chain(
                    spec.constraints
                        .iter()
                        .filter_map(|constraint| match constraint.kind {
                            SketchConstraintKind::Projection { source_feature, .. } => {
                                Some(source_feature)
                            }
                            _ => None,
                        }),
                )
                .collect(),
            Self::Profile { .. }
            | Self::SheetMetal(_)
            | Self::SpatialPath { .. }
            | Self::ConstructionPoint { .. }
            | Self::ConstructionAxis { .. }
            | Self::ConstructionPlane { .. }
            | Self::ImportedExactBody(_)
            | Self::MeshBody(_) => BTreeSet::new(),
            Self::RigidTransform { target, .. } => [*target].into_iter().collect(),
            Self::Revolve { profile, .. } | Self::PlanarOffset { profile, .. } => {
                [*profile].into_iter().collect()
            }
            Self::Pad(spec) => spec
                .operation
                .target()
                .into_iter()
                .chain([spec.profile.feature_id()])
                .collect(),
            Self::Shell { target, .. }
            | Self::EdgeFinish { target, .. }
            | Self::FaceOffset { target, .. } => [*target].into_iter().collect(),
            Self::Boolean { target, tool, .. } => [*target, *tool].into_iter().collect(),
            Self::Sweep { profile, path, .. } => [*profile, *path].into_iter().collect(),
            Self::WeldmentMember(spec) => [spec.profile, spec.path].into_iter().collect(),
            Self::WeldmentJoint(spec) => [spec.first_member, spec.second_member]
                .into_iter()
                .collect(),
            Self::Loft {
                sections, guide, ..
            }
            | Self::SurfaceBody(SurfaceBodySpec::Loft {
                sections, guide, ..
            }) => sections
                .iter()
                .map(|section| section.profile)
                .chain(*guide)
                .collect(),
            Self::SurfaceBody(SurfaceBodySpec::Planar { profile }) => {
                [*profile].into_iter().collect()
            }
            Self::SurfaceTrim { target, cutter } => [*target, *cutter].into_iter().collect(),
            Self::SurfaceExtend { target, .. } | Self::SurfaceThicken { target, .. } => {
                [*target].into_iter().collect()
            }
            Self::SurfaceKnit { surfaces, .. } => surfaces.iter().copied().collect(),
        }
    }

    #[must_use]
    pub fn authoritative_dependencies(&self) -> BTreeSet<FeatureId> {
        let mut dependencies = self.dependencies();
        let references = match self {
            Self::Pad(spec) => spec.references(),
            _ => Vec::new(),
        };
        for reference in references {
            dependencies.insert(reference.profile_feature_id);
            dependencies.insert(reference.producer_feature_id);
        }
        dependencies
    }

    #[must_use]
    pub const fn body_kind(&self) -> Option<BodyKind> {
        match self {
            Self::SurfaceBody(_)
            | Self::SurfaceTrim { .. }
            | Self::SurfaceExtend { .. }
            | Self::PlanarOffset { .. } => Some(BodyKind::Surface),
            Self::SurfaceKnit { make_solid, .. } => Some(if *make_solid {
                BodyKind::Solid
            } else {
                BodyKind::Surface
            }),
            Self::SurfaceThicken { .. } => Some(BodyKind::Solid),
            Self::Pad(_)
            | Self::Revolve { .. }
            | Self::Shell { .. }
            | Self::EdgeFinish { .. }
            | Self::FaceOffset { .. }
            | Self::Boolean { .. }
            | Self::Sweep { .. }
            | Self::WeldmentMember(_)
            | Self::WeldmentJoint(_)
            | Self::Loft { .. }
            | Self::SheetMetal(_)
            | Self::RigidTransform { .. }
            | Self::MeshBody(_) => Some(BodyKind::Solid),
            Self::ImportedExactBody(spec) => Some(spec.body_kind),
            _ => None,
        }
    }

    #[must_use]
    pub const fn produces_body(&self) -> bool {
        self.body_kind().is_some()
    }

    #[must_use]
    pub const fn full_revolve(profile: FeatureId) -> Self {
        Self::Revolve {
            profile,
            axis_start_mm: [0.0, 0.0],
            axis_end_mm: [0.0, 1.0],
            angle_degrees: 360.0,
        }
    }
}

pub(super) fn push_parameter_descriptor(
    descriptors: &mut Vec<ParameterDescriptor>,
    path: impl Into<String>,
    value_type: ParameterValueType,
) {
    descriptors.push(
        ParameterDescriptor::new(path, value_type)
            .expect("feature-derived parameter paths are canonical and bounded"),
    );
}

pub(super) fn describe_feature_extent(
    descriptors: &mut Vec<ParameterDescriptor>,
    prefix: &str,
    extent: &ketchup_geometry::sketch::FeatureExtent,
) {
    match extent {
        ketchup_geometry::sketch::FeatureExtent::Blind(_)
        | ketchup_geometry::sketch::FeatureExtent::Symmetric(_) => {
            push_parameter_descriptor(
                descriptors,
                format!("{prefix}.distance"),
                ParameterValueType::Length,
            );
        }
        ketchup_geometry::sketch::FeatureExtent::Bidirectional { along, opposite } => {
            describe_feature_extent_end(descriptors, &format!("{prefix}.along"), along);
            describe_feature_extent_end(descriptors, &format!("{prefix}.opposite"), opposite);
        }
        ketchup_geometry::sketch::FeatureExtent::ThroughAll
        | ketchup_geometry::sketch::FeatureExtent::UpToFace(_) => {}
    }
}

pub(super) fn describe_feature_extent_end(
    descriptors: &mut Vec<ParameterDescriptor>,
    prefix: &str,
    extent: &ketchup_geometry::sketch::FeatureExtentEnd,
) {
    if matches!(extent, ketchup_geometry::sketch::FeatureExtentEnd::Blind(_)) {
        push_parameter_descriptor(
            descriptors,
            format!("{prefix}.distance"),
            ParameterValueType::Length,
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Body {
    pub(crate) id: BodyId,
    pub(crate) name: String,
    pub(crate) visible: bool,
    pub(crate) consumed_by: Option<FeatureId>,
}

impl Body {
    #[must_use]
    pub const fn id(&self) -> BodyId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }

    #[must_use]
    pub const fn consumed_by(&self) -> Option<FeatureId> {
        self.consumed_by
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FeatureBodyOwnership {
    pub(super) input_body_ids: Vec<BodyId>,
    pub(super) output_body_id: Option<BodyId>,
}

impl FeatureBodyOwnership {
    pub fn new(
        input_body_ids: Vec<BodyId>,
        output_body_id: Option<BodyId>,
    ) -> Result<Self, CanonicalError> {
        if input_body_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(CanonicalError::BodyInputsNotCanonical);
        }
        Ok(Self {
            input_body_ids,
            output_body_id,
        })
    }

    #[must_use]
    pub fn input_body_ids(&self) -> &[BodyId] {
        &self.input_body_ids
    }

    #[must_use]
    pub const fn output_body_id(&self) -> Option<BodyId> {
        self.output_body_id
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Feature {
    pub(crate) id: FeatureId,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) kind: FeatureKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureDependencyGraph {
    pub(super) dependencies: BTreeMap<FeatureId, BTreeSet<FeatureId>>,
    pub(super) dependents: BTreeMap<FeatureId, BTreeSet<FeatureId>>,
    pub(super) topological_order: Vec<FeatureId>,
}

impl FeatureDependencyGraph {
    pub(super) fn from_product(product: &ProductModel) -> Result<Self, CanonicalError> {
        let mut dependencies = BTreeMap::new();
        let mut dependents = product
            .features
            .keys()
            .cloned()
            .map(|id| (id, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        let mut indegree = BTreeMap::new();
        for (id, feature) in &product.features {
            let material_dependencies = feature.kind.dependencies();
            let mut feature_dependencies = feature.kind.authoritative_dependencies();
            if !material_dependencies.contains(id) {
                feature_dependencies.remove(id);
            }
            for dependency in &feature_dependencies {
                let source = product
                    .features
                    .get(dependency)
                    .ok_or(CanonicalError::FeatureNotFound(*dependency))?;
                if source.definition_id != feature.definition_id {
                    return Err(CanonicalError::InvalidFeatureOwnership(*id));
                }
                dependents.entry(*dependency).or_default().insert(*id);
            }
            indegree.insert(*id, feature_dependencies.len());
            dependencies.insert(*id, feature_dependencies);
        }

        let mut ready = indegree
            .iter()
            .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
            .collect::<BTreeSet<_>>();
        let mut topological_order = Vec::with_capacity(indegree.len());
        while let Some(id) = ready.pop_first() {
            topological_order.push(id);
            for dependent in &dependents[&id] {
                let degree = indegree
                    .get_mut(dependent)
                    .expect("every dependent is a feature");
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(*dependent);
                }
            }
        }
        if topological_order.len() != indegree.len() {
            let cycle = indegree
                .into_iter()
                .filter_map(|(id, degree)| (degree != 0).then_some(id))
                .min()
                .expect("a rejected graph contains a cycle");
            return Err(
                if product
                    .features
                    .get(&cycle)
                    .is_some_and(|feature| matches!(feature.kind, FeatureKind::Workplane(_)))
                {
                    CanonicalError::Sketch(SketchError::WorkplaneCycle(cycle))
                } else {
                    CanonicalError::FeatureDependencyCycle(cycle)
                },
            );
        }
        Ok(Self {
            dependencies,
            dependents,
            topological_order,
        })
    }

    #[must_use]
    pub fn dependencies(&self, id: FeatureId) -> Option<&BTreeSet<FeatureId>> {
        self.dependencies.get(&id)
    }

    #[must_use]
    pub fn dependents(&self, id: FeatureId) -> Option<&BTreeSet<FeatureId>> {
        self.dependents.get(&id)
    }

    #[must_use]
    pub fn topological_order(&self) -> &[FeatureId] {
        &self.topological_order
    }

    #[must_use]
    pub fn dependent_closure(
        &self,
        roots: impl IntoIterator<Item = FeatureId>,
    ) -> BTreeSet<FeatureId> {
        let mut closure = roots.into_iter().collect::<BTreeSet<_>>();
        let mut pending = closure.iter().cloned().collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            if let Some(dependents) = self.dependents.get(&id) {
                for dependent in dependents {
                    if closure.insert(*dependent) {
                        pending.push(*dependent);
                    }
                }
            }
        }
        closure
    }

    #[must_use]
    pub fn evaluation_states(
        &self,
        stale: &BTreeSet<FeatureId>,
        errors: &BTreeSet<FeatureId>,
    ) -> BTreeMap<FeatureId, FeatureEvaluationState> {
        let mut states = BTreeMap::new();
        for id in &self.topological_order {
            let failed_dependency =
                self.dependencies[id]
                    .iter()
                    .find_map(|dependency| match states.get(dependency) {
                        Some(FeatureEvaluationState::Error { failed_at }) => Some(*failed_at),
                        _ => None,
                    });
            let state = if errors.contains(id) {
                FeatureEvaluationState::Error { failed_at: *id }
            } else if let Some(failed_at) = failed_dependency {
                FeatureEvaluationState::Error { failed_at }
            } else if stale.contains(id)
                || self.dependencies[id].iter().any(|dependency| {
                    states.get(dependency) == Some(&FeatureEvaluationState::Stale)
                })
            {
                FeatureEvaluationState::Stale
            } else {
                FeatureEvaluationState::Current
            };
            states.insert(*id, state);
        }
        states
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureEvaluationState {
    Current,
    Stale,
    Error { failed_at: FeatureId },
}

impl Feature {
    #[must_use]
    pub const fn id(&self) -> FeatureId {
        self.id
    }

    #[must_use]
    pub const fn definition_id(&self) -> DefinitionId {
        self.definition_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> &FeatureKind {
        &self.kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Definition {
    pub(crate) id: DefinitionId,
    pub(crate) name: String,
    pub(crate) feature_ids: Vec<FeatureId>,
    pub(crate) bodies: BTreeMap<BodyId, Body>,
    pub(crate) active_body_id: BodyId,
    pub(crate) feature_body_ownership: BTreeMap<FeatureId, FeatureBodyOwnership>,
    pub(crate) local_occurrence_ids: Vec<LocalOccurrenceId>,
    pub(crate) local_group_ids: Vec<LocalGroupId>,
}

impl Definition {
    #[must_use]
    pub const fn id(&self) -> DefinitionId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn feature_ids(&self) -> &[FeatureId] {
        &self.feature_ids
    }

    pub fn bodies(&self) -> impl Iterator<Item = &Body> {
        self.bodies.values()
    }

    #[must_use]
    pub fn body(&self, id: BodyId) -> Option<&Body> {
        self.bodies.get(&id)
    }

    #[must_use]
    pub const fn active_body_id(&self) -> BodyId {
        self.active_body_id
    }

    #[must_use]
    pub fn feature_body_ownership(&self, id: FeatureId) -> Option<&FeatureBodyOwnership> {
        self.feature_body_ownership.get(&id)
    }

    #[must_use]
    pub fn local_occurrence_ids(&self) -> &[LocalOccurrenceId] {
        &self.local_occurrence_ids
    }

    #[must_use]
    pub fn local_group_ids(&self) -> &[LocalGroupId] {
        &self.local_group_ids
    }
}

pub(super) const DEFAULT_BODY_ID: BodyId = BodyId(1);

pub(super) fn default_body() -> Body {
    Body {
        id: DEFAULT_BODY_ID,
        name: "Body".to_owned(),
        visible: true,
        consumed_by: None,
    }
}

pub(super) fn new_definition(id: DefinitionId, name: String) -> Definition {
    Definition {
        id,
        name,
        feature_ids: Vec::new(),
        bodies: BTreeMap::from([(DEFAULT_BODY_ID, default_body())]),
        active_body_id: DEFAULT_BODY_ID,
        feature_body_ownership: BTreeMap::new(),
        local_occurrence_ids: Vec::new(),
        local_group_ids: Vec::new(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tag {
    pub(crate) id: TagId,
    pub(crate) name: String,
    pub(crate) visible: bool,
}

impl Tag {
    #[must_use]
    pub const fn id(&self) -> TagId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClassificationCategory {
    pub(crate) id: ClassificationCategoryId,
    pub(crate) name: String,
}

impl ClassificationCategory {
    #[must_use]
    pub const fn id(&self) -> ClassificationCategoryId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClassificationDimension {
    pub(crate) id: ClassificationDimensionId,
    pub(crate) name: String,
    pub(crate) categories: BTreeMap<ClassificationCategoryId, ClassificationCategory>,
}

impl ClassificationDimension {
    #[must_use]
    pub const fn id(&self) -> ClassificationDimensionId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn categories(&self) -> impl Iterator<Item = &ClassificationCategory> {
        self.categories.values()
    }

    #[must_use]
    pub fn category(&self, id: ClassificationCategoryId) -> Option<&ClassificationCategory> {
        self.categories.get(&id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Collection {
    pub(crate) id: CollectionId,
    pub(crate) name: String,
    pub(crate) occurrence_ids: BTreeSet<OccurrenceId>,
}

impl Collection {
    #[must_use]
    pub const fn id(&self) -> CollectionId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn occurrence_ids(&self) -> impl Iterator<Item = OccurrenceId> + '_ {
        self.occurrence_ids.iter().cloned()
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Occurrence {
    pub(crate) id: OccurrenceId,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<GroupId>,
    pub(crate) tag: Option<TagId>,
    pub(crate) visible: bool,
    pub(crate) color: Option<[u8; 3]>,
}

impl Occurrence {
    /// Optional sRGB appearance override; never changes geometry.
    #[must_use]
    pub const fn color(&self) -> Option<[u8; 3]> {
        self.color
    }

    #[must_use]
    pub const fn id(&self) -> OccurrenceId {
        self.id
    }

    #[must_use]
    pub const fn definition_id(&self) -> DefinitionId {
        self.definition_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn parent(&self) -> Option<GroupId> {
        self.parent
    }

    #[must_use]
    pub const fn tag(&self) -> Option<TagId> {
        self.tag
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Group {
    pub(crate) id: GroupId,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<GroupId>,
}

impl Group {
    #[must_use]
    pub const fn id(&self) -> GroupId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn parent(&self) -> Option<GroupId> {
        self.parent
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocalOccurrence {
    pub(crate) key: LocalOccurrenceKey,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<LocalGroupId>,
    pub(crate) tag: Option<TagId>,
    pub(crate) visible: bool,
    pub(crate) color: Option<[u8; 3]>,
}

impl LocalOccurrence {
    /// Optional sRGB appearance override; never changes geometry.
    #[must_use]
    pub const fn color(&self) -> Option<[u8; 3]> {
        self.color
    }

    #[must_use]
    pub const fn key(&self) -> LocalOccurrenceKey {
        self.key
    }

    #[must_use]
    pub const fn definition_id(&self) -> DefinitionId {
        self.definition_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn parent(&self) -> Option<LocalGroupId> {
        self.parent
    }

    #[must_use]
    pub const fn tag(&self) -> Option<TagId> {
        self.tag
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocalGroup {
    pub(crate) key: LocalGroupKey,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<LocalGroupId>,
}

impl LocalGroup {
    #[must_use]
    pub const fn key(&self) -> LocalGroupKey {
        self.key
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn parent(&self) -> Option<LocalGroupId> {
        self.parent
    }
}

#[cfg(test)]
mod transform_tests {
    use super::*;

    fn close(left: [f64; 3], right: [f64; 3]) -> bool {
        left.iter()
            .zip(right)
            .all(|(l, r)| (l - r).abs() <= 1.0e-12)
    }

    /// A quarter turn about z, a non-uniform scale and an offset, so rows
    /// and columns and points and directions cannot be confused.
    fn sample() -> Transform {
        Transform::from_matrix([
            0.0, -2.0, 0.0, 10.0, 1.0, 0.0, 0.0, 20.0, 0.0, 0.0, 3.0, 30.0, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap()
    }

    #[test]
    fn points_take_the_offset_and_directions_do_not() {
        assert_eq!(sample().transform_point([1.0, 2.0, 3.0]), [6.0, 21.0, 39.0]);
        assert_eq!(sample().transform_vector([1.0, 2.0, 3.0]), [-4.0, 1.0, 9.0]);
    }

    #[test]
    fn compose_applies_the_local_transform_first_and_inverse_undoes_it() {
        let local = Transform::from_translation(1.0, 2.0, 3.0).unwrap();
        let point = [4.0, -5.0, 6.0];
        let composed = sample().compose(local);
        let expected = sample().transform_point(local.transform_point(point));
        assert!(close(composed.transform_point(point), expected));
        let back = composed.inverse().unwrap().transform_point(expected);
        assert!(close(back, point));
        assert_eq!(sample().rigid_inverse(), None);
    }
}
