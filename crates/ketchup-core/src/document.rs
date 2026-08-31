use crate::assembly::{
    ASSEMBLY_MATE_SCHEMA_V1, AssemblyDofDiagnostic, AssemblyDofStatus, AssemblyMate,
    AssemblyMateId, AssemblyMateKind, AssemblyReferenceHealth,
};
use crate::assembly_joint::{
    ASSEMBLY_JOINT_SCHEMA_V1, ASSEMBLY_MOTION_STUDY_SCHEMA_V1, AssemblyJoint, AssemblyJointId,
    AssemblyJointKind, AssemblyJointLimits, AssemblyMotionStudy, AssemblyMotionStudyId,
    joint_motion_states_equal, solve_assembly_joint_kinematics_with_kind_overrides,
    transforms_equivalent,
};
use crate::bottle_m6::{ExactRevolveRequest, reference_matches_revolve_request};
use crate::drawing::{DrawingError, DrawingSheet, DrawingSheetId, DrawingSource};
use crate::exact_product::{
    BodySubshapeRef, ExactFaceRole, ExactFeatureChainRequest, ExactReferenceResolution,
    ExactResultRegistry, is_line_arc_capsule_profile, line_arc_capsule_corner_overlap,
    line_arc_capsule_profile_bounds, line_arc_capsule_side_overlap, line_arc_circle_side_overlap,
    line_arc_d_arc_only_side_overlap, strict_convex_line_arc_profile_bounds,
};
pub use crate::graph::{
    CanonicalOverride, DerivedIdentity, DerivedOutput, EvaluationIdentity, EvaluationReport,
    EvaluationStatus, EvaluatorNode, EvaluatorNodeKind, GraphError, OverrideMergePolicy,
    OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution, SlotSegment, ValueType,
};
use crate::graph::{
    ExpressionAst, evaluate_affected, evaluate_graph, resolve_derived_identity,
    validate_graph as validate_typed_graph,
};
use crate::import::{
    ImportDiagnosticSeverity, ImportFormat, ImportId, ImportLengthUnit, ImportOutputRef,
    ImportReceipt, ImportUnitAuthority,
};
use crate::mechanical_coupling::{
    ASSEMBLY_MOTION_COUPLING_SCHEMA_V1, AssemblyMotionCoupling, AssemblyMotionCouplingId,
    CoupledJointKind,
};
use crate::prismatic::{CanonicalJoint, JointId, PrismaticError};
use crate::sketch::{
    FeatureExtent, FeatureExtentEnd, PadPocketOperation, PadSpec, PocketSpec, PrincipalPlane,
    SketchConstraintId, SketchConstraintKind, SketchEntity, SketchError, SketchPointKind,
    SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport, WorkplaneSupportHealth,
};
use crate::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceCoordinateFrame, ClearanceOwner,
    ClearanceSeverity, ClearanceVolumeId, SpaceError, SpaceId,
};
use crate::topology::{TopologicalElementKind, TopologicalElementRef};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const COMMAND_SCHEMA_V1: &str = "ketchup.command.v1";
pub const TOLERANCE_PROFILE_V1: &str = "ketchup.tolerance.r0-v1";
const MAX_CANONICAL_ABS_MM: f64 = 1_000_000.0;

macro_rules! typed_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);
    };
}

typed_id!(DocumentId);
typed_id!(DefinitionId);
typed_id!(BodyId);
typed_id!(OccurrenceId);
typed_id!(GroupId);
typed_id!(FeatureId);
typed_id!(TagId);
typed_id!(ClassificationDimensionId);
typed_id!(ClassificationCategoryId);
typed_id!(CollectionId);
typed_id!(NodeId);
typed_id!(LocalOccurrenceId);
typed_id!(LocalGroupId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalOccurrenceKey {
    pub definition_id: DefinitionId,
    pub local_id: LocalOccurrenceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalGroupKey {
    pub definition_id: DefinitionId,
    pub local_id: LocalGroupId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstancePath {
    root: OccurrenceId,
    steps: Vec<InstancePathStep>,
}

impl InstancePath {
    #[must_use]
    pub const fn root(root: OccurrenceId) -> Self {
        Self {
            root,
            steps: Vec::new(),
        }
    }

    #[must_use]
    pub const fn root_occurrence(&self) -> OccurrenceId {
        self.root
    }

    #[must_use]
    pub fn steps(&self) -> &[InstancePathStep] {
        &self.steps
    }

    #[must_use]
    pub fn with_step(&self, step: InstancePathStep) -> Self {
        let mut path = self.clone();
        path.steps.push(step);
        path
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.steps.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InstancePathStep {
    Group(LocalGroupId),
    Occurrence(LocalOccurrenceId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitSystem {
    Millimetres,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    matrix: [f64; 16],
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
        let mut result = [0.0; 16];
        for row in 0..4 {
            for column in 0..4 {
                result[row * 4 + column] = (0..4)
                    .map(|index| self.matrix[row * 4 + index] * local.matrix[index * 4 + column])
                    .sum();
            }
        }
        Self { matrix: result }
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BottleControlDimension {
    BodyRadius,
    BodyHeight,
    ShoulderRise,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BottleEdgeFinishKind {
    Fillet,
    Chamfer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdgeFinishKind {
    Fillet,
    Chamfer,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StableFaceRole(String);

impl StableFaceRole {
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StableEdgeRole(String);

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

pub const BOTTLE_SHELL_OPENING_FACE_ROLE: &str = "revolve.mouth";
pub const BOTTLE_SHOULDER_EDGE_ROLE: &str = "shell.edge.shoulder";
pub const MAX_PARAMETER_PATH_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ParameterPath(String);

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

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ParameterValueType {
    Length,
    Angle,
    Scalar,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterDescriptor {
    path: ParameterPath,
    value_type: ParameterValueType,
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

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureParameterBinding {
    pub target: FeatureParameterTarget,
    pub derived_from: DerivedIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BooleanOperation {
    Cut,
    Union,
    Intersect,
    Split,
}

pub const MESH_BODY_SCHEMA_V1: &str = "ketchup.mesh-body.v1";
pub const IMPORTED_EXACT_BODY_SCHEMA_V1: &str = "ketchup.imported-exact-body.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactReferenceConversionConsequence {
    Lost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeshAuthority {
    Authored { provenance: String },
    ExactConversion(ExactToMeshConversion),
    ImportedStl { import_id: ImportId },
    ImportedSketchupScene { import_id: ImportId },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedExactBodySpec {
    pub schema: String,
    pub import_id: ImportId,
    pub source_sha256: [u8; 32],
    pub source_byte_len: u64,
    pub result_fingerprint: String,
    pub solid_count: u32,
    pub topology_counts: Option<[u32; 5]>,
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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
}

impl ProfileSegment {
    #[must_use]
    pub const fn start_mm(&self) -> [f64; 2] {
        match self {
            Self::Line { start_mm, .. } | Self::CircularArc { start_mm, .. } => *start_mm,
        }
    }

    #[must_use]
    pub const fn end_mm(&self) -> [f64; 2] {
        match self {
            Self::Line { end_mm, .. } | Self::CircularArc { end_mm, .. } => *end_mm,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoftSection {
    pub profile: FeatureId,
    pub elevation_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureKind {
    Workplane(WorkplaneSpec),
    Sketch(SketchSpec),
    Profile {
        points_mm: Vec<[f64; 2]>,
    },
    SegmentProfile {
        segments: Vec<ProfileSegment>,
        closed: bool,
    },
    SplineProfile {
        control_points_mm: Vec<[f64; 2]>,
    },
    Extrusion {
        profile: FeatureId,
        height: Dimension,
    },
    Pad(PadSpec),
    SketchPocket(PocketSpec),
    BottleProfileControl {
        profile: FeatureId,
        body_radius: Dimension,
        body_height: Dimension,
        shoulder_rise: Dimension,
    },
    Revolve {
        profile: FeatureId,
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    },
    Shell {
        target: FeatureId,
        removed_faces: Vec<StableFaceRole>,
        thickness: Dimension,
    },
    BottleEdgeFinish {
        target: FeatureId,
        edges: Vec<StableEdgeRole>,
        kind: BottleEdgeFinishKind,
        amount: Dimension,
    },
    TopologyShell {
        target: FeatureId,
        removed_faces: Vec<TopologicalElementRef>,
        thickness: Dimension,
    },
    TopologyEdgeFinish {
        target: FeatureId,
        edges: Vec<TopologicalElementRef>,
        kind: EdgeFinishKind,
        amount: Dimension,
    },
    TopologyFaceOffset {
        target: FeatureId,
        face: TopologicalElementRef,
        distance: Dimension,
    },
    ThroughCut {
        target: FeatureId,
        profile: FeatureId,
    },
    Pocket {
        target: FeatureId,
        profile: FeatureId,
        depth: Dimension,
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
    },
    Loft {
        sections: Vec<LoftSection>,
    },
    ImportedExactBody(ImportedExactBodySpec),
    MeshBody(MeshBodySpec),
}

impl FeatureKind {
    #[must_use]
    pub fn parameter_descriptors(&self) -> Vec<ParameterDescriptor> {
        let mut descriptors = Vec::new();
        match self {
            Self::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Offset { .. },
                ..
            }) => push_parameter_descriptor(
                &mut descriptors,
                "support.offset.distance",
                ParameterValueType::Length,
            ),
            Self::Sketch(spec) => {
                for entity in &spec.entities {
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
                        | SketchConstraintKind::Coincident { .. } => {}
                    }
                }
            }
            Self::Profile { points_mm } => {
                if is_axis_aligned_rectangle(points_mm) {
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
                for (index, _) in points_mm.iter().enumerate() {
                    for axis in ["x", "y"] {
                        push_parameter_descriptor(
                            &mut descriptors,
                            format!("points.{index}.{axis}"),
                            ParameterValueType::Length,
                        );
                    }
                }
            }
            Self::SegmentProfile { segments, .. } => {
                for (index, segment) in segments.iter().enumerate() {
                    for point in ["start", "end"] {
                        for axis in ["x", "y"] {
                            push_parameter_descriptor(
                                &mut descriptors,
                                format!("segments.{index}.{point}.{axis}"),
                                ParameterValueType::Length,
                            );
                        }
                    }
                    if matches!(segment, ProfileSegment::CircularArc { .. }) {
                        for axis in ["x", "y"] {
                            push_parameter_descriptor(
                                &mut descriptors,
                                format!("segments.{index}.center.{axis}"),
                                ParameterValueType::Length,
                            );
                        }
                    }
                }
            }
            Self::SplineProfile { control_points_mm } => {
                for (index, _) in control_points_mm.iter().enumerate() {
                    for axis in ["x", "y"] {
                        push_parameter_descriptor(
                            &mut descriptors,
                            format!("control_points.{index}.{axis}"),
                            ParameterValueType::Length,
                        );
                    }
                }
            }
            Self::Extrusion { .. } => {
                push_parameter_descriptor(&mut descriptors, "height", ParameterValueType::Length)
            }
            Self::Pad(spec) => describe_feature_extent(&mut descriptors, "extent", &spec.extent),
            Self::SketchPocket(spec) => {
                describe_feature_extent(&mut descriptors, "extent", &spec.extent);
            }
            Self::BottleProfileControl { .. } => {
                for path in ["body_radius", "body_height", "shoulder_rise"] {
                    push_parameter_descriptor(&mut descriptors, path, ParameterValueType::Length);
                }
            }
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
            Self::Shell { .. } | Self::TopologyShell { .. } => {
                push_parameter_descriptor(&mut descriptors, "thickness", ParameterValueType::Length)
            }
            Self::BottleEdgeFinish { .. } | Self::TopologyEdgeFinish { .. } => {
                push_parameter_descriptor(&mut descriptors, "amount", ParameterValueType::Length);
            }
            Self::TopologyFaceOffset { .. } | Self::PlanarOffset { .. } => {
                push_parameter_descriptor(&mut descriptors, "distance", ParameterValueType::Length);
            }
            Self::Pocket { .. } => {
                push_parameter_descriptor(&mut descriptors, "depth", ParameterValueType::Length)
            }
            Self::Loft { sections } => {
                for (index, _) in sections.iter().enumerate() {
                    push_parameter_descriptor(
                        &mut descriptors,
                        format!("sections.{index}.elevation"),
                        ParameterValueType::Length,
                    );
                }
            }
            Self::ThroughCut { .. }
            | Self::Boolean { .. }
            | Self::Sweep { .. }
            | Self::ImportedExactBody(_)
            | Self::MeshBody(_)
            | Self::Workplane(_) => {}
        }
        descriptors
    }

    #[must_use]
    pub fn dependencies(&self) -> BTreeSet<FeatureId> {
        match self {
            Self::Workplane(spec) => match &spec.support {
                WorkplaneSupport::Principal(_) => BTreeSet::new(),
                WorkplaneSupport::Offset { base, .. } => [*base].into_iter().collect(),
                WorkplaneSupport::PlanarFace { reference, .. } => {
                    [reference.profile_feature_id, reference.producer_feature_id]
                        .into_iter()
                        .collect()
                }
            },
            Self::Sketch(spec) => [spec.workplane].into_iter().collect(),
            Self::Profile { .. }
            | Self::SegmentProfile { .. }
            | Self::SplineProfile { .. }
            | Self::ImportedExactBody(_)
            | Self::MeshBody(_) => BTreeSet::new(),
            Self::Extrusion { profile, .. }
            | Self::BottleProfileControl { profile, .. }
            | Self::Revolve { profile, .. }
            | Self::PlanarOffset { profile, .. } => [*profile].into_iter().collect(),
            Self::Pad(spec) => [spec.sketch].into_iter().collect(),
            Self::SketchPocket(spec) => [spec.target, spec.sketch].into_iter().collect(),
            Self::Shell { target, .. }
            | Self::BottleEdgeFinish { target, .. }
            | Self::TopologyShell { target, .. }
            | Self::TopologyEdgeFinish { target, .. }
            | Self::TopologyFaceOffset { target, .. } => [*target].into_iter().collect(),
            Self::ThroughCut { target, profile }
            | Self::Pocket {
                target, profile, ..
            } => [*target, *profile].into_iter().collect(),
            Self::Boolean { target, tool, .. } => [*target, *tool].into_iter().collect(),
            Self::Sweep { profile, path } => [*profile, *path].into_iter().collect(),
            Self::Loft { sections } => sections.iter().map(|section| section.profile).collect(),
        }
    }

    #[must_use]
    pub fn authoritative_dependencies(&self) -> BTreeSet<FeatureId> {
        let mut dependencies = self.dependencies();
        let references = match self {
            Self::Pad(spec) => spec.extent.references(),
            Self::SketchPocket(spec) => std::iter::once(spec.support.as_ref())
                .chain(spec.extent.references())
                .collect(),
            _ => Vec::new(),
        };
        for reference in references {
            dependencies.insert(reference.profile_feature_id);
            dependencies.insert(reference.producer_feature_id);
        }
        dependencies
    }

    #[must_use]
    pub const fn produces_body(&self) -> bool {
        matches!(
            self,
            Self::Extrusion { .. }
                | Self::Pad(_)
                | Self::SketchPocket(_)
                | Self::Revolve { .. }
                | Self::Shell { .. }
                | Self::BottleEdgeFinish { .. }
                | Self::TopologyShell { .. }
                | Self::TopologyEdgeFinish { .. }
                | Self::TopologyFaceOffset { .. }
                | Self::ThroughCut { .. }
                | Self::Pocket { .. }
                | Self::Boolean { .. }
                | Self::Sweep { .. }
                | Self::Loft { .. }
                | Self::ImportedExactBody(_)
                | Self::MeshBody(_)
        )
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

fn push_parameter_descriptor(
    descriptors: &mut Vec<ParameterDescriptor>,
    path: impl Into<String>,
    value_type: ParameterValueType,
) {
    descriptors.push(
        ParameterDescriptor::new(path, value_type)
            .expect("feature-derived parameter paths are canonical and bounded"),
    );
}

fn describe_feature_extent(
    descriptors: &mut Vec<ParameterDescriptor>,
    prefix: &str,
    extent: &crate::sketch::FeatureExtent,
) {
    match extent {
        crate::sketch::FeatureExtent::Blind(_) | crate::sketch::FeatureExtent::Symmetric(_) => {
            push_parameter_descriptor(
                descriptors,
                format!("{prefix}.distance"),
                ParameterValueType::Length,
            );
        }
        crate::sketch::FeatureExtent::Bidirectional { along, opposite } => {
            describe_feature_extent_end(descriptors, &format!("{prefix}.along"), along);
            describe_feature_extent_end(descriptors, &format!("{prefix}.opposite"), opposite);
        }
        crate::sketch::FeatureExtent::ThroughAll | crate::sketch::FeatureExtent::UpToFace(_) => {}
    }
}

fn describe_feature_extent_end(
    descriptors: &mut Vec<ParameterDescriptor>,
    prefix: &str,
    extent: &crate::sketch::FeatureExtentEnd,
) {
    if matches!(extent, crate::sketch::FeatureExtentEnd::Blind(_)) {
        push_parameter_descriptor(
            descriptors,
            format!("{prefix}.distance"),
            ParameterValueType::Length,
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureBodyOwnership {
    input_body_ids: Vec<BodyId>,
    output_body_id: Option<BodyId>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub(crate) id: FeatureId,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) kind: FeatureKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureDependencyGraph {
    dependencies: BTreeMap<FeatureId, BTreeSet<FeatureId>>,
    dependents: BTreeMap<FeatureId, BTreeSet<FeatureId>>,
    topological_order: Vec<FeatureId>,
}

impl FeatureDependencyGraph {
    fn from_product(product: &ProductModel) -> Result<Self, CanonicalError> {
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

const DEFAULT_BODY_ID: BodyId = BodyId(1);

fn default_body() -> Body {
    Body {
        id: DEFAULT_BODY_ID,
        name: "Body".to_owned(),
        visible: true,
        consumed_by: None,
    }
}

fn new_definition(id: DefinitionId, name: String) -> Definition {
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub(crate) id: OccurrenceId,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<GroupId>,
    pub(crate) tag: Option<TagId>,
    pub(crate) visible: bool,
}

impl Occurrence {
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct LocalOccurrence {
    pub(crate) key: LocalOccurrenceKey,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<LocalGroupId>,
    pub(crate) tag: Option<TagId>,
    pub(crate) visible: bool,
}

impl LocalOccurrence {
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone)]
pub(crate) struct ProductModel {
    pub(crate) document_id: DocumentId,
    pub(crate) units: UnitSystem,
    pub(crate) evaluator_nodes: BTreeMap<NodeId, Arc<EvaluatorNode>>,
    pub(crate) overrides: BTreeMap<u64, Arc<CanonicalOverride>>,
    pub(crate) feature_parameter_bindings:
        BTreeMap<FeatureParameterTarget, Arc<FeatureParameterBinding>>,
    pub(crate) feature_parameter_provenance:
        BTreeMap<FeatureParameterTarget, Arc<FeatureParameterProvenance>>,
    pub(crate) joints: BTreeMap<JointId, Arc<CanonicalJoint>>,
    pub(crate) spaces: BTreeMap<SpaceId, Arc<CanonicalSpace>>,
    pub(crate) clearance_volumes: BTreeMap<ClearanceVolumeId, Arc<CanonicalClearanceVolume>>,
    pub(crate) exact_reference_evidence: BTreeMap<String, Arc<BodySubshapeRef>>,
    pub(crate) persistent_dimensions: BTreeMap<PersistentDimensionId, Arc<PersistentDimension>>,
    pub(crate) tags: BTreeMap<TagId, Arc<Tag>>,
    pub(crate) classification_dimensions:
        BTreeMap<ClassificationDimensionId, Arc<ClassificationDimension>>,
    pub(crate) classification_assignments:
        BTreeMap<(OccurrenceId, ClassificationDimensionId), ClassificationCategoryId>,
    pub(crate) collections: BTreeMap<CollectionId, Arc<Collection>>,
    pub(crate) import_receipts: BTreeMap<ImportId, Arc<ImportReceipt>>,
    pub(crate) definitions: BTreeMap<DefinitionId, Arc<Definition>>,
    pub(crate) features: BTreeMap<FeatureId, Arc<Feature>>,
    pub(crate) body_feature_suppression: BTreeMap<(DefinitionId, BodyId), BTreeSet<FeatureId>>,
    pub(crate) occurrences: BTreeMap<OccurrenceId, Arc<Occurrence>>,
    pub(crate) grounded_occurrences: BTreeSet<OccurrenceId>,
    pub(crate) assembly_mates: BTreeMap<AssemblyMateId, Arc<AssemblyMate>>,
    pub(crate) assembly_joints: BTreeMap<AssemblyJointId, Arc<AssemblyJoint>>,
    pub(crate) assembly_motion_couplings:
        BTreeMap<AssemblyMotionCouplingId, Arc<AssemblyMotionCoupling>>,
    pub(crate) assembly_motion_studies: BTreeMap<AssemblyMotionStudyId, Arc<AssemblyMotionStudy>>,
    pub(crate) drawing_sheets: BTreeMap<DrawingSheetId, Arc<DrawingSheet>>,
    pub(crate) groups: BTreeMap<GroupId, Arc<Group>>,
    pub(crate) local_occurrences: BTreeMap<LocalOccurrenceKey, Arc<LocalOccurrence>>,
    pub(crate) local_groups: BTreeMap<LocalGroupKey, Arc<LocalGroup>>,
    pub(crate) canonical_digest: DigestCache,
}

/// The canonical digest of one immutable product model, computed at most once.
///
/// Hashing the whole document is O(document), and interactive paths ask for the
/// digest many times per frame to check whether a derived result is still
/// current. A product model never changes after it is published in a snapshot,
/// so the digest can be memoized. Cloning a model means a new revision is being
/// built from it, so the clone starts with an empty cache.
#[derive(Default)]
pub(crate) struct DigestCache(OnceLock<String>);

impl Clone for DigestCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Default for ProductModel {
    fn default() -> Self {
        Self {
            document_id: allocate_document_id(),
            units: UnitSystem::Millimetres,
            evaluator_nodes: BTreeMap::new(),
            overrides: BTreeMap::new(),
            feature_parameter_bindings: BTreeMap::new(),
            feature_parameter_provenance: BTreeMap::new(),
            joints: BTreeMap::new(),
            spaces: BTreeMap::new(),
            clearance_volumes: BTreeMap::new(),
            exact_reference_evidence: BTreeMap::new(),
            persistent_dimensions: BTreeMap::new(),
            tags: BTreeMap::new(),
            classification_dimensions: BTreeMap::new(),
            classification_assignments: BTreeMap::new(),
            collections: BTreeMap::new(),
            import_receipts: BTreeMap::new(),
            definitions: BTreeMap::new(),
            features: BTreeMap::new(),
            body_feature_suppression: BTreeMap::new(),
            occurrences: BTreeMap::new(),
            grounded_occurrences: BTreeSet::new(),
            assembly_mates: BTreeMap::new(),
            assembly_joints: BTreeMap::new(),
            assembly_motion_couplings: BTreeMap::new(),
            assembly_motion_studies: BTreeMap::new(),
            drawing_sheets: BTreeMap::new(),
            groups: BTreeMap::new(),
            local_occurrences: BTreeMap::new(),
            local_groups: BTreeMap::new(),
            canonical_digest: DigestCache::default(),
        }
    }
}

fn allocate_document_id() -> DocumentId {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |duration| (duration.as_nanos() as u64).max(1));
    let value = NEXT
        .get_or_init(|| AtomicU64::new(seed))
        .fetch_add(1, Ordering::Relaxed);
    DocumentId(value.max(1))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneQueryContext {
    Group(GroupId),
    Definition {
        definition_id: DefinitionId,
        instance_path: InstancePath,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundSceneQuery {
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    context: SceneQueryContext,
}

impl BoundSceneQuery {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub const fn context(&self) -> &SceneQueryContext {
        &self.context
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneQueryError {
    InvalidContext,
    SnapshotMismatch,
}

impl fmt::Display for SceneQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidContext => "scene query context is invalid or hidden",
            Self::SnapshotMismatch => "scene query is bound to a different snapshot",
        })
    }
}

impl std::error::Error for SceneQueryError {}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneOccurrence {
    pub occurrence_id: OccurrenceId,
    pub instance_path: InstancePath,
    pub definition_id: DefinitionId,
    pub occurrence_name: String,
    pub definition_name: String,
    pub transform: Transform,
    pub parent: Option<GroupId>,
    pub local_parent: Option<LocalGroupId>,
    pub visible: bool,
    pub shared_occurrence_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    source_token: String,
    millimetres: f64,
}

impl Dimension {
    pub fn new(source_token: impl Into<String>, millimetres: f64) -> Result<Self, CanonicalError> {
        let source_token = source_token.into();
        if source_token.trim().is_empty() {
            return Err(CanonicalError::EmptySourceToken);
        }
        if !millimetres.is_finite() || millimetres.abs() > MAX_CANONICAL_ABS_MM {
            return Err(CanonicalError::DimensionOutsideEnvelope);
        }
        Ok(Self {
            source_token,
            millimetres,
        })
    }

    pub fn from_decimal(source_token: impl Into<String>) -> Result<Self, CanonicalError> {
        let source_token = source_token.into();
        let millimetres = source_token
            .parse::<f64>()
            .map_err(|_| CanonicalError::InvalidDecimalToken)?;
        Self::new(source_token, millimetres)
    }

    #[must_use]
    pub fn source_token(&self) -> &str {
        &self.source_token
    }

    #[must_use]
    pub const fn millimetres(&self) -> f64 {
        self.millimetres
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PersistentDimensionId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersistentDimensionTarget {
    FeatureParameter(FeatureParameterTarget),
    DerivedOutput(DerivedIdentity),
    ExactFeatureParameter {
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
        semantic_role: String,
        source_element_id: String,
        path: ParameterPath,
        value_type: ParameterValueType,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DimensionDisplayUnit {
    Millimetres,
    Centimetres,
    Inches,
}

impl DimensionDisplayUnit {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Millimetres => "mm",
            Self::Centimetres => "cm",
            Self::Inches => "in",
        }
    }

    #[must_use]
    pub fn from_millimetres(self, millimetres: f64) -> f64 {
        match self {
            Self::Millimetres => millimetres,
            Self::Centimetres => millimetres / 10.0,
            Self::Inches => millimetres / 25.4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DimensionPresentation {
    pub unit: DimensionDisplayUnit,
    pub decimal_places: u8,
}

impl DimensionPresentation {
    pub fn new(unit: DimensionDisplayUnit, decimal_places: u8) -> Result<Self, CanonicalError> {
        if decimal_places > 9 {
            return Err(CanonicalError::InvalidDimensionPresentation);
        }
        Ok(Self {
            unit,
            decimal_places,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistentDimension {
    pub id: PersistentDimensionId,
    pub name: String,
    pub target: PersistentDimensionTarget,
    pub presentation: DimensionPresentation,
}

impl PersistentDimension {
    pub fn new(
        id: PersistentDimensionId,
        name: impl Into<String>,
        target: PersistentDimensionTarget,
        presentation: DimensionPresentation,
    ) -> Result<Self, CanonicalError> {
        ensure_product_id(id.0)?;
        let name = name.into();
        ensure_name(&name)?;
        if matches!(
            &target,
            PersistentDimensionTarget::ExactFeatureParameter {
                definition_id,
                producer_feature_id,
                semantic_role,
                source_element_id,
                ..
            } if definition_id.0 == 0
                || producer_feature_id.0 == 0
                || semantic_role.is_empty()
                || source_element_id.is_empty()
        ) {
            return Err(CanonicalError::InvalidPersistentDimensionTarget);
        }
        Ok(Self {
            id,
            name,
            target,
            presentation,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DimensionReferenceHealth {
    Resolved,
    Ambiguous { segment_index: usize },
    Lost,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PersistentDimensionProjection {
    pub id: PersistentDimensionId,
    pub health: DimensionReferenceHealth,
    pub millimetres: Option<f64>,
    pub display_value: Option<f64>,
    pub display_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalCommand {
    CreateEvaluatorNode {
        id: NodeId,
        name: String,
        dimension: Dimension,
        dependencies: Vec<NodeId>,
    },
    SetEvaluatorDimension {
        id: NodeId,
        dimension: Dimension,
    },
    RenameEvaluatorNode {
        id: NodeId,
        name: String,
    },
    CreateExpressionNode {
        id: NodeId,
        name: String,
        expression: String,
    },
    CreateRuleNode {
        id: NodeId,
        name: String,
        expression: String,
        input_ports: Vec<PortSpec>,
        output_ports: Vec<PortSpec>,
        outputs: Vec<RuleOutput>,
        override_parameters: Vec<OverrideParameterSpec>,
    },
    SetNodeExpression {
        id: NodeId,
        expression: String,
    },
    SetRuleOutputs {
        id: NodeId,
        outputs: Vec<RuleOutput>,
    },
    UpsertOverride(CanonicalOverride),
    DeleteOverride {
        id: u64,
    },
    UpsertFeatureParameterBinding(FeatureParameterBinding),
    DeleteFeatureParameterBinding {
        target: FeatureParameterTarget,
    },
    RecomputeFeatureParameters {
        identity: EvaluationIdentity,
    },
    UpsertJoint(CanonicalJoint),
    DeleteJoint {
        id: JointId,
    },
    UpsertSpace(CanonicalSpace),
    DeleteSpace {
        id: SpaceId,
    },
    UpsertClearanceVolume(CanonicalClearanceVolume),
    DeleteClearanceVolume {
        id: ClearanceVolumeId,
    },
    UpsertPersistentDimension(PersistentDimension),
    DeletePersistentDimension {
        id: PersistentDimensionId,
    },
    CreateTag {
        id: TagId,
        name: String,
        visible: bool,
    },
    DeleteTag {
        id: TagId,
    },
    SetTagVisibility {
        id: TagId,
        visible: bool,
    },
    SetTagName {
        id: TagId,
        name: String,
    },
    UpsertClassificationDimension {
        id: ClassificationDimensionId,
        name: String,
        categories: Vec<(ClassificationCategoryId, String)>,
    },
    SetOccurrenceClassification {
        occurrence_id: OccurrenceId,
        dimension_id: ClassificationDimensionId,
        category_id: Option<ClassificationCategoryId>,
    },
    CreateCollection {
        id: CollectionId,
        name: String,
    },
    DeleteCollection {
        id: CollectionId,
    },
    SetCollectionOccurrences {
        id: CollectionId,
        occurrence_ids: Vec<OccurrenceId>,
    },
    RecordImport(ImportReceipt),
    CreateDefinition {
        id: DefinitionId,
        name: String,
    },
    DeleteDefinition {
        id: DefinitionId,
    },
    RenameDefinition {
        id: DefinitionId,
        name: String,
    },
    CreateBody {
        definition_id: DefinitionId,
        id: BodyId,
        name: String,
        visible: bool,
    },
    DeleteBody {
        definition_id: DefinitionId,
        id: BodyId,
    },
    RenameBody {
        definition_id: DefinitionId,
        id: BodyId,
        name: String,
    },
    SetActiveBody {
        definition_id: DefinitionId,
        id: BodyId,
    },
    SetBodyVisibility {
        definition_id: DefinitionId,
        id: BodyId,
        visible: bool,
    },
    ConsumeBody {
        definition_id: DefinitionId,
        id: BodyId,
        by_feature_id: FeatureId,
    },
    SetFeatureBodyOwnership {
        id: FeatureId,
        ownership: FeatureBodyOwnership,
    },
    SetBodyFeatureSuppression {
        definition_id: DefinitionId,
        body_id: BodyId,
        suppressed_feature_ids: Vec<FeatureId>,
    },
    CreateFeature {
        id: FeatureId,
        definition_id: DefinitionId,
        name: String,
        kind: FeatureKind,
    },
    DeleteFeature {
        id: FeatureId,
    },
    SetFeatureDimension {
        id: FeatureId,
        dimension: Dimension,
    },
    SetSketchConstraintDimension {
        id: FeatureId,
        constraint_id: SketchConstraintId,
        dimension: Dimension,
    },
    TranslateProfile {
        id: FeatureId,
        delta_mm: [f64; 2],
    },
    SetBottleControlDimension {
        id: FeatureId,
        control: BottleControlDimension,
        dimension: Dimension,
    },
    SetBottleEdgeFinishKind {
        id: FeatureId,
        kind: BottleEdgeFinishKind,
    },
    SetProfilePoints {
        id: FeatureId,
        points_mm: Vec<[f64; 2]>,
    },
    CreateOccurrence {
        id: OccurrenceId,
        definition_id: DefinitionId,
        name: String,
        transform: Transform,
        parent: Option<GroupId>,
        tag: Option<TagId>,
        visible: bool,
    },
    DeleteOccurrence {
        id: OccurrenceId,
    },
    SetOccurrenceTransform {
        id: OccurrenceId,
        transform: Transform,
    },
    RenameEntity {
        id: OccurrenceId,
        name: String,
    },
    ApplyAssemblySolve {
        source_revision: u64,
        source_digest: String,
        transforms: Vec<(OccurrenceId, Transform)>,
    },
    GuardAssemblyRecompute {
        source_revision: u64,
        source_digest: String,
    },
    SetOccurrenceGrounded {
        id: OccurrenceId,
        grounded: bool,
    },
    CreateAssemblyMate(AssemblyMate),
    RebindAssemblyMate(AssemblyMate),
    SetAssemblyMateKind {
        id: AssemblyMateId,
        kind: AssemblyMateKind,
    },
    DeleteAssemblyMate {
        id: AssemblyMateId,
    },
    CreateAssemblyJoint(AssemblyJoint),
    SetAssemblyJointKind {
        id: AssemblyJointId,
        kind: AssemblyJointKind,
    },
    SetAssemblyJointPosition {
        id: AssemblyJointId,
        position: f64,
    },
    SetAssemblyJointLimits {
        id: AssemblyJointId,
        limits: Option<AssemblyJointLimits>,
    },
    DeleteAssemblyJoint {
        id: AssemblyJointId,
    },
    CreateAssemblyMotionCoupling(AssemblyMotionCoupling),
    UpdateAssemblyMotionCoupling(AssemblyMotionCoupling),
    DeleteAssemblyMotionCoupling {
        id: AssemblyMotionCouplingId,
    },
    CreateAssemblyMotionStudy(AssemblyMotionStudy),
    UpdateAssemblyMotionStudy(AssemblyMotionStudy),
    DeleteAssemblyMotionStudy {
        id: AssemblyMotionStudyId,
    },
    CreateDrawingSheet(DrawingSheet),
    UpdateDrawingSheet(DrawingSheet),
    DeleteDrawingSheet {
        id: DrawingSheetId,
    },
    SetOccurrenceVisibility {
        id: OccurrenceId,
        visible: bool,
    },
    SetOccurrenceTag {
        id: OccurrenceId,
        tag: Option<TagId>,
    },
    RepointOccurrence {
        id: OccurrenceId,
        definition_id: DefinitionId,
    },
    SetOccurrenceParent {
        id: OccurrenceId,
        parent: Option<GroupId>,
    },
    CreateGroup {
        id: GroupId,
        name: String,
        transform: Transform,
        parent: Option<GroupId>,
    },
    DeleteGroup {
        id: GroupId,
    },
    SetGroupTransform {
        id: GroupId,
        transform: Transform,
    },
    SetGroupParent {
        id: GroupId,
        parent: Option<GroupId>,
    },
    CloneDefinitionAndRepoint(CloneDefinitionPlan),
    ConvertGroupToComponent(ConvertGroupPlan),
    ApplySolidTool(SolidToolPlan),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CloneDefinitionPlan {
    occurrence_id: OccurrenceId,
    source_definition_id: DefinitionId,
    new_definition_id: DefinitionId,
    new_definition_name: String,
    feature_id_map: Vec<(FeatureId, FeatureId)>,
}

impl CloneDefinitionPlan {
    pub fn new(
        occurrence_id: OccurrenceId,
        source_definition_id: DefinitionId,
        new_definition_id: DefinitionId,
        new_definition_name: String,
        feature_id_map: Vec<(FeatureId, FeatureId)>,
    ) -> Self {
        Self {
            occurrence_id,
            source_definition_id,
            new_definition_id,
            new_definition_name,
            feature_id_map,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvertGroupPlan {
    group_id: GroupId,
    new_definition_id: DefinitionId,
    new_occurrence_id: OccurrenceId,
    component_name: String,
}

impl ConvertGroupPlan {
    pub(crate) fn new(
        group_id: GroupId,
        new_definition_id: DefinitionId,
        new_occurrence_id: OccurrenceId,
        component_name: String,
    ) -> Self {
        Self {
            group_id,
            new_definition_id,
            new_occurrence_id,
            component_name,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SolidToolPlan {
    pub operation: BooleanOperation,
    pub target_occurrence_id: OccurrenceId,
    pub target_feature_id: FeatureId,
    pub tool_occurrence_id: OccurrenceId,
    pub tool_feature_id: FeatureId,
    pub result_definition_id: DefinitionId,
    pub result_feature_ids: [FeatureId; 5],
    pub result_definition_name: String,
    pub result_feature_name: String,
    pub keep_tool: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewBodyFeaturePlan {
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub body_name: String,
    pub feature_id: FeatureId,
    pub feature_name: String,
    pub feature_kind: FeatureKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolBodyPolicy {
    Preserve,
    Consume,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiBodyBooleanPlan {
    pub definition_id: DefinitionId,
    pub operation: BooleanOperation,
    pub target_body_id: BodyId,
    pub target_feature_id: FeatureId,
    pub tool_body_id: BodyId,
    pub tool_feature_id: FeatureId,
    pub result_feature_id: FeatureId,
    pub result_feature_name: String,
    pub tool_policy: ToolBodyPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthoritativeDependency {
    EvaluatorNode(NodeId),
    Override(u64),
    FeatureParameterBinding(FeatureParameterTarget),
    Joint(JointId),
    Space(SpaceId),
    ClearanceVolume(ClearanceVolumeId),
    PersistentDimension(PersistentDimensionId),
    Tag(TagId),
    ClassificationDimension(ClassificationDimensionId),
    OccurrenceClassification(OccurrenceId, ClassificationDimensionId),
    Collection(CollectionId),
    Import(ImportId),
    Definition(DefinitionId),
    Feature(FeatureId),
    BodyFeatureSuppression(DefinitionId, BodyId),
    Occurrence(OccurrenceId),
    GroundedOccurrence(OccurrenceId),
    AssemblyMate(AssemblyMateId),
    AssemblyJoint(AssemblyJointId),
    AssemblyMotionCoupling(AssemblyMotionCouplingId),
    AssemblyMotionStudy(AssemblyMotionStudyId),
    DrawingSheet(DrawingSheetId),
    Group(GroupId),
    LocalGroup(LocalGroupKey),
    LocalOccurrence(LocalOccurrenceKey),
    DefinitionUsers(DefinitionId),
    FeatureUsers(FeatureId),
    FeatureParameterBindings(FeatureId),
    GroupChildren(GroupId),
    GroupSubtree(GroupId),
    OccurrenceCollections(OccurrenceId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProposalBudget {
    pub max_commands: usize,
    pub max_read_dependencies: usize,
    pub max_write_targets: usize,
}

impl ProposalBudget {
    pub const HOST_MAX: Self = Self {
        max_commands: 512,
        max_read_dependencies: 1_024,
        max_write_targets: 1_024,
    };

    pub const M7A_SINGLE_CHANGE: Self = Self {
        max_commands: 1,
        max_read_dependencies: 64,
        max_write_targets: 1,
    };

    pub const M18C_CREATE_FEATURE: Self = Self {
        max_commands: 1,
        max_read_dependencies: 64,
        max_write_targets: 2,
    };

    pub const M18C_CLONE_PROFILE_DEFINITION: Self = Self {
        max_commands: 1,
        max_read_dependencies: 64,
        max_write_targets: 3,
    };

    pub const T18_ATOMIC_MULTI_COMMAND_EDIT: Self = Self {
        max_commands: 3,
        max_read_dependencies: 64,
        max_write_targets: 1,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProposalCost {
    pub commands: usize,
    pub read_dependencies: usize,
    pub write_targets: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalPrincipal {
    ManualClient,
    Human(u64),
    LocalAssistant,
    Plugin(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighRiskClass {
    DestructiveBulkChange,
    Overwrite,
    LossyConversion,
    ExternalDisclosure,
    ReleaseManufacturingExportWithWarnings,
    CapabilityExpansion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalRisk {
    Standard,
    High(HighRiskClass),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighRiskScope {
    class: HighRiskClass,
    destination: Option<String>,
    provider: Option<String>,
    path: Option<String>,
}

impl HighRiskScope {
    pub fn new(
        class: HighRiskClass,
        destination: Option<String>,
        provider: Option<String>,
        path: Option<String>,
    ) -> Result<Self, HumanConfirmationError> {
        for value in [destination.as_deref(), provider.as_deref(), path.as_deref()]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
                return Err(HumanConfirmationError::InvalidScope);
            }
        }
        if matches!(class, HighRiskClass::ExternalDisclosure)
            && (destination.is_none() || provider.is_none())
        {
            return Err(HumanConfirmationError::InvalidScope);
        }
        if matches!(
            class,
            HighRiskClass::Overwrite
                | HighRiskClass::LossyConversion
                | HighRiskClass::ReleaseManufacturingExportWithWarnings
        ) && path.is_none()
        {
            return Err(HumanConfirmationError::InvalidScope);
        }
        Ok(Self {
            class,
            destination,
            provider,
            path,
        })
    }

    #[must_use]
    pub const fn class(&self) -> HighRiskClass {
        self.class
    }

    #[must_use]
    pub fn destination(&self) -> Option<&str> {
        self.destination.as_deref()
    }

    #[must_use]
    pub fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalConfirmation {
    ReviewRequired,
    HumanOnly(HighRiskScope),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalGoal {
    CanonicalPreview,
    CreateEvaluatorInput(NodeId),
    CreateEvaluatorExpression(NodeId),
    CreateEvaluatorRule(NodeId),
    CreateRuleOverride(u64),
    DeleteRuleOverride(u64),
    CreateFeatureParameterBinding(FeatureParameterTarget),
    DeleteFeatureParameterBinding(FeatureParameterTarget),
    CreatePersistentDimension(PersistentDimensionId),
    CreateSpace(SpaceId),
    CreateClearanceVolume(ClearanceVolumeId),
    CreateJoint(JointId),
    RecomputeFeatureParameter(FeatureParameterTarget),
    DeleteJoint(JointId),
    DeleteSpace(SpaceId),
    DeleteClearanceVolume(ClearanceVolumeId),
    DeletePersistentDimension(PersistentDimensionId),
    SetRuleDimension(NodeId),
    RenameEvaluatorNode(NodeId),
    SetEvaluatorExpression(NodeId),
    SetRuleOutputs(NodeId),
    SetFeatureDimension(FeatureId),
    SetBottleControlDimension(FeatureId, BottleControlDimension),
    SetBottleEdgeFinishKind(FeatureId),
    SetProfilePoints(FeatureId),
    RenameDefinition(DefinitionId),
    SetOccurrenceVisibility(OccurrenceId),
    SetOccurrenceTranslation(OccurrenceId),
    AtomicMultiCommandEdit(OccurrenceId),
    SetOccurrenceTag(OccurrenceId),
    SetTagVisibility(TagId),
    RepointOccurrence(OccurrenceId),
    SetOccurrenceParent(OccurrenceId),
    SetGroupTranslation(GroupId),
    SetGroupParent(GroupId),
    SetCollectionOccurrences(CollectionId),
    CreateTag(TagId),
    DeleteTag(TagId),
    CreateCollection(CollectionId),
    DeleteCollection(CollectionId),
    DeleteGroup(GroupId),
    DeleteOccurrence(OccurrenceId),
    CreateDefinition(DefinitionId),
    DeleteDefinition(DefinitionId),
    CreateProfileFeature(FeatureId),
    DeleteProfileFeature(FeatureId),
    CreateGroup(GroupId),
    CreateOccurrence(OccurrenceId),
    CloneProfileDefinitionAndRepoint(OccurrenceId),
    ConvertEmptyGroupToComponent(GroupId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalAssumption {
    TargetExists(AuthoritativeDependency),
    TargetMissing(AuthoritativeDependency),
    TargetHasDimension(AuthoritativeDependency),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProposalValue {
    Missing,
    EvaluatorInputState {
        name: String,
        dimension: Dimension,
        dependencies: Vec<NodeId>,
    },
    EvaluatorExpressionState {
        name: String,
        expression: String,
        dependencies: Vec<NodeId>,
    },
    EvaluatorRuleState {
        name: String,
        expression: String,
        dependencies: Vec<NodeId>,
        input_ports: Vec<PortSpec>,
        output_ports: Vec<PortSpec>,
        outputs: Vec<RuleOutput>,
        override_parameters: Vec<OverrideParameterSpec>,
    },
    RuleOverrideState {
        target: DerivedIdentity,
        parameter: String,
        value: f64,
        health: SlotResolution,
    },
    FeatureParameterBindingState {
        target: FeatureParameterTarget,
        derived_from: DerivedIdentity,
    },
    JointState {
        participant_a: DerivedIdentity,
        participant_b: DerivedIdentity,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
    },
    SpaceState {
        purpose: String,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
        adjacent_to: Vec<SpaceId>,
        accessible_to: Vec<SpaceId>,
    },
    ClearanceVolumeState {
        owner: ClearanceOwner,
        reason: String,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
        coordinate_frame: ClearanceCoordinateFrame,
        tolerance_mm: f64,
        severity: ClearanceSeverity,
        derived_from: Option<DerivedIdentity>,
    },
    PersistentDimensionState {
        name: String,
        target: PersistentDimensionTarget,
        presentation: DimensionPresentation,
    },
    Boolean(bool),
    Dimension(Dimension),
    BottleEdgeFinishKind(BottleEdgeFinishKind),
    RuleOutputs(Vec<RuleOutput>),
    ProfilePoints(Vec<[f64; 2]>),
    Transform(Transform),
    Tag(Option<TagId>),
    TagState {
        name: String,
        visible: bool,
    },
    CollectionState {
        name: String,
        occurrence_ids: Vec<OccurrenceId>,
    },
    Definition(DefinitionId),
    DefinitionState {
        name: String,
        feature_ids: Vec<FeatureId>,
        local_occurrence_ids: Vec<LocalOccurrenceId>,
        local_group_ids: Vec<LocalGroupId>,
    },
    DefinitionFeatures(Vec<FeatureId>),
    ProfileFeatureState {
        definition: DefinitionId,
        name: String,
        points_mm: Vec<[f64; 2]>,
    },
    Group(Option<GroupId>),
    GroupState {
        name: String,
        transform: Transform,
        parent: Option<GroupId>,
    },
    OccurrenceState {
        definition: DefinitionId,
        name: String,
        transform: Transform,
        parent: Option<GroupId>,
        tag: Option<TagId>,
        visible: bool,
    },
    Occurrences(Vec<OccurrenceId>),
    Text(String),
    Digest(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProposalDiffEntry {
    pub target: AuthoritativeDependency,
    pub before: ProposalValue,
    pub after: ProposalValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalContext {
    pub principal: ProposalPrincipal,
    pub goal: ProposalGoal,
    pub assumptions: Vec<ProposalAssumption>,
    pub risk: ProposalRisk,
    pub confirmation: ProposalConfirmation,
    pub requested_budget: ProposalBudget,
}

impl ProposalContext {
    #[must_use]
    pub fn local_assistant_model() -> Self {
        Self {
            principal: ProposalPrincipal::LocalAssistant,
            goal: ProposalGoal::CanonicalPreview,
            assumptions: Vec::new(),
            risk: ProposalRisk::Standard,
            confirmation: ProposalConfirmation::ReviewRequired,
            requested_budget: ProposalBudget::HOST_MAX,
        }
    }

    #[must_use]
    pub fn canonical_preview() -> Self {
        Self {
            principal: ProposalPrincipal::ManualClient,
            goal: ProposalGoal::CanonicalPreview,
            assumptions: Vec::new(),
            risk: ProposalRisk::Standard,
            confirmation: ProposalConfirmation::ReviewRequired,
            requested_budget: ProposalBudget::HOST_MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorldEntityId {
    Group(GroupId),
    Occurrence(OccurrenceId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertedEntityId {
    ComponentOccurrence(OccurrenceId),
    LocalGroup(LocalGroupKey),
    LocalOccurrence(LocalOccurrenceKey),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorldEntityPath {
    pub groups: Vec<GroupId>,
    pub occurrence: Option<OccurrenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnresolvedMappingReason {
    NotInConvertedGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MappingResolution {
    Resolved {
        new_id: ConvertedEntityId,
        new_path: InstancePath,
    },
    Unresolved {
        reason: UnresolvedMappingReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionMapping {
    pub old_id: WorldEntityId,
    pub old_path: WorldEntityPath,
    pub resolution: MappingResolution,
}

pub struct ConvertGroupToComponentResult {
    pub revision: Arc<Revision>,
    pub component_definition_id: DefinitionId,
    pub component_occurrence_id: OccurrenceId,
    pub mappings: Vec<ConversionMapping>,
}

impl ConvertGroupToComponentResult {
    #[must_use]
    pub fn resolve_old_path(&self, old_path: &WorldEntityPath) -> MappingResolution {
        self.mappings
            .iter()
            .find(|mapping| &mapping.old_path == old_path)
            .map_or(
                MappingResolution::Unresolved {
                    reason: UnresolvedMappingReason::NotInConvertedGroup,
                },
                |mapping| mapping.resolution.clone(),
            )
    }

    pub fn unresolved_mappings(&self) -> impl Iterator<Item = &ConversionMapping> {
        self.mappings
            .iter()
            .filter(|mapping| matches!(mapping.resolution, MappingResolution::Unresolved { .. }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandBatch {
    schema: &'static str,
    commands: Vec<CanonicalCommand>,
}

impl CommandBatch {
    #[must_use]
    pub fn new(commands: Vec<CanonicalCommand>) -> Self {
        Self {
            schema: COMMAND_SCHEMA_V1,
            commands,
        }
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.schema
    }

    #[must_use]
    pub fn commands(&self) -> &[CanonicalCommand] {
        &self.commands
    }

    #[must_use]
    pub fn digest(&self) -> String {
        let mut digest = StableDigest::new();
        digest.bytes(self.schema.as_bytes());
        digest.u64(self.commands.len() as u64);
        for command in &self.commands {
            digest.command(command);
        }
        digest.finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedInstance {
    pub definition_id: DefinitionId,
    pub world_transform: Transform,
}

#[derive(Clone)]
pub struct Snapshot {
    revision_id: u64,
    product: Arc<ProductModel>,
}

impl Snapshot {
    pub(crate) fn preview_batch(&self, batch: &CommandBatch) -> Result<Self, CanonicalError> {
        let mut candidate =
            DocumentStore::from_product(self.revision_id, self.product.as_ref().clone())?;
        candidate.apply_batch(batch)?;
        Ok(candidate.current())
    }

    #[must_use]
    pub const fn revision_id(&self) -> u64 {
        self.revision_id
    }

    #[must_use]
    pub fn evaluator_node(&self, id: NodeId) -> Option<&EvaluatorNode> {
        self.product.evaluator_nodes.get(&id).map(Arc::as_ref)
    }

    pub fn evaluator_nodes(&self) -> impl Iterator<Item = &EvaluatorNode> {
        self.product.evaluator_nodes.values().map(Arc::as_ref)
    }

    pub fn evaluator_node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.product.evaluator_nodes.keys().cloned()
    }

    #[must_use]
    pub fn evaluator_node_count(&self) -> usize {
        self.product.evaluator_nodes.len()
    }

    #[must_use]
    pub fn shares_evaluator_node_with(&self, other: &Self, id: NodeId) -> bool {
        match (
            self.product.evaluator_nodes.get(&id),
            other.product.evaluator_nodes.get(&id),
        ) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub fn overrides(&self) -> impl Iterator<Item = &CanonicalOverride> {
        self.product.overrides.values().map(Arc::as_ref)
    }

    pub fn feature_parameter_bindings(&self) -> impl Iterator<Item = &FeatureParameterBinding> {
        self.product
            .feature_parameter_bindings
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn feature_parameter_binding(
        &self,
        target: &FeatureParameterTarget,
    ) -> Option<&FeatureParameterBinding> {
        self.product
            .feature_parameter_bindings
            .get(target)
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn has_feature_parameter(&self, target: &FeatureParameterTarget) -> bool {
        feature_parameter_dimension(&self.product, target).is_some()
    }

    #[must_use]
    pub fn feature_parameter_provenance(
        &self,
        target: &FeatureParameterTarget,
    ) -> Option<&FeatureParameterProvenance> {
        self.product
            .feature_parameter_provenance
            .get(target)
            .map(Arc::as_ref)
    }

    pub fn audit_feature_parameter_freshness(
        &self,
        identity: &EvaluationIdentity,
    ) -> Result<Vec<FeatureParameterFreshnessAudit>, CanonicalError> {
        let report = evaluate_graph(&self.product.evaluator_nodes, identity)
            .map_err(CanonicalError::Graph)?;
        self.product
            .feature_parameter_bindings
            .values()
            .map(|binding| {
                let freshness =
                    audit_feature_parameter_binding(&self.product, binding, identity, &report);
                Ok(FeatureParameterFreshnessAudit {
                    target: binding.target.clone(),
                    freshness,
                })
            })
            .collect()
    }

    #[must_use]
    pub fn override_by_id(&self, id: u64) -> Option<&CanonicalOverride> {
        self.product.overrides.get(&id).map(Arc::as_ref)
    }

    pub fn joints(&self) -> impl Iterator<Item = &CanonicalJoint> {
        self.product.joints.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn joint(&self, id: JointId) -> Option<&CanonicalJoint> {
        self.product.joints.get(&id).map(Arc::as_ref)
    }

    pub fn spaces(&self) -> impl Iterator<Item = &CanonicalSpace> {
        self.product.spaces.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn space(&self, id: SpaceId) -> Option<&CanonicalSpace> {
        self.product.spaces.get(&id).map(Arc::as_ref)
    }

    pub fn clearance_volumes(&self) -> impl Iterator<Item = &CanonicalClearanceVolume> {
        self.product.clearance_volumes.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn clearance_volume(&self, id: ClearanceVolumeId) -> Option<&CanonicalClearanceVolume> {
        self.product.clearance_volumes.get(&id).map(Arc::as_ref)
    }

    pub fn exact_reference_evidence(&self) -> impl Iterator<Item = &BodySubshapeRef> {
        self.product
            .exact_reference_evidence
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn exact_reference_by_lineage(&self, lineage_digest: &str) -> Option<&BodySubshapeRef> {
        self.product
            .exact_reference_evidence
            .get(lineage_digest)
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn resolved_planar_face_workplane_frame(
        &self,
        reference: &BodySubshapeRef,
    ) -> Option<WorkplaneFrame> {
        self.exact_reference_by_lineage(&reference.lineage_digest)
            .filter(|evidence| *evidence == reference)?;
        supported_planar_face_frame(&self.product, reference)
    }

    pub fn persistent_dimensions(&self) -> impl Iterator<Item = &PersistentDimension> {
        self.product.persistent_dimensions.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn persistent_dimension(&self, id: PersistentDimensionId) -> Option<&PersistentDimension> {
        self.product.persistent_dimensions.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn project_persistent_dimension(
        &self,
        id: PersistentDimensionId,
    ) -> Option<PersistentDimensionProjection> {
        let dimension = self.persistent_dimension(id)?;
        let (health, millimetres) = resolve_persistent_dimension(self.product(), dimension);
        let display_value =
            millimetres.map(|value| dimension.presentation.unit.from_millimetres(value));
        let display_text = display_value.map(|value| {
            format!(
                "{value:.precision$} {}",
                dimension.presentation.unit.label(),
                precision = usize::from(dimension.presentation.decimal_places)
            )
        });
        Some(PersistentDimensionProjection {
            id,
            health,
            millimetres,
            display_value,
            display_text,
        })
    }

    #[must_use]
    pub fn resolve_slot(&self, identity: &DerivedIdentity) -> SlotResolution {
        resolve_derived_identity(&self.product.evaluator_nodes, identity)
    }

    pub fn evaluate(
        &self,
        identity: &EvaluationIdentity,
    ) -> Result<EvaluationReport, CanonicalError> {
        let mut report = evaluate_graph(&self.product.evaluator_nodes, identity)
            .map_err(CanonicalError::Graph)?;
        report.document_id = Some(self.document_id());
        report.revision_id = Some(self.revision_id());
        report.canonical_digest = Some(self.canonical_digest());
        Ok(report)
    }

    #[must_use]
    pub fn canonical_digest(&self) -> String {
        self.product
            .canonical_digest
            .0
            .get_or_init(|| digest_snapshot(self))
            .clone()
    }

    #[must_use]
    pub fn document_id(&self) -> DocumentId {
        self.product.document_id
    }

    pub fn feature_dependency_graph(&self) -> Result<FeatureDependencyGraph, CanonicalError> {
        FeatureDependencyGraph::from_product(&self.product)
    }

    #[must_use]
    pub fn units(&self) -> UnitSystem {
        self.product.units
    }

    #[must_use]
    pub fn tag(&self, id: TagId) -> Option<&Tag> {
        self.product.tags.get(&id).map(Arc::as_ref)
    }

    pub fn tags(&self) -> impl Iterator<Item = &Tag> {
        self.product.tags.values().map(Arc::as_ref)
    }

    pub fn occurrences_with_tag(&self, id: TagId) -> impl Iterator<Item = &Occurrence> {
        self.product
            .occurrences
            .values()
            .filter(move |occurrence| occurrence.tag == Some(id))
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn classification_dimension(
        &self,
        id: ClassificationDimensionId,
    ) -> Option<&ClassificationDimension> {
        self.product
            .classification_dimensions
            .get(&id)
            .map(Arc::as_ref)
    }

    pub fn classification_dimensions(&self) -> impl Iterator<Item = &ClassificationDimension> {
        self.product
            .classification_dimensions
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn occurrence_classification(
        &self,
        occurrence_id: OccurrenceId,
        dimension_id: ClassificationDimensionId,
    ) -> Option<ClassificationCategoryId> {
        self.product
            .classification_assignments
            .get(&(occurrence_id, dimension_id))
            .cloned()
    }

    pub fn occurrence_classifications(
        &self,
        occurrence_id: OccurrenceId,
    ) -> impl Iterator<Item = (ClassificationDimensionId, ClassificationCategoryId)> + '_ {
        self.product
            .classification_assignments
            .range(
                (occurrence_id, ClassificationDimensionId(0))
                    ..=(occurrence_id, ClassificationDimensionId(u64::MAX)),
            )
            .map(|((_, dimension_id), category_id)| (*dimension_id, *category_id))
    }

    #[must_use]
    pub fn collection(&self, id: CollectionId) -> Option<&Collection> {
        self.product.collections.get(&id).map(Arc::as_ref)
    }

    pub fn collections(&self) -> impl Iterator<Item = &Collection> {
        self.product.collections.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn import_receipt(&self, id: ImportId) -> Option<&ImportReceipt> {
        self.product.import_receipts.get(&id).map(Arc::as_ref)
    }

    pub fn import_receipts(&self) -> impl Iterator<Item = &ImportReceipt> {
        self.product.import_receipts.values().map(Arc::as_ref)
    }

    pub fn next_import_id(&self) -> Result<ImportId, CanonicalError> {
        next_id(self.product.import_receipts.keys().map(|id| id.0)).map(ImportId)
    }

    pub fn occurrences_in_collection(&self, id: CollectionId) -> impl Iterator<Item = &Occurrence> {
        self.product
            .collections
            .get(&id)
            .into_iter()
            .flat_map(|collection| collection.occurrence_ids.iter())
            .filter_map(|occurrence_id| self.product.occurrences.get(occurrence_id))
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn occurrence_effectively_visible(&self, id: OccurrenceId) -> Option<bool> {
        let occurrence = self.occurrence(id)?;
        Some(
            occurrence.visible
                && occurrence
                    .tag
                    .and_then(|tag_id| self.tag(tag_id))
                    .is_none_or(Tag::visible),
        )
    }

    #[must_use]
    pub fn definition(&self, id: DefinitionId) -> Option<&Definition> {
        self.product.definitions.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.product.features.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn suppressed_feature_ids(
        &self,
        definition_id: DefinitionId,
        body_id: BodyId,
    ) -> Option<&BTreeSet<FeatureId>> {
        self.product
            .body_feature_suppression
            .get(&(definition_id, body_id))
    }

    #[must_use]
    pub fn feature_is_suppressed(&self, id: FeatureId) -> bool {
        self.product
            .body_feature_suppression
            .values()
            .any(|suppressed| suppressed.contains(&id))
    }

    #[must_use]
    pub fn occurrence(&self, id: OccurrenceId) -> Option<&Occurrence> {
        self.product.occurrences.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn occurrence_is_grounded(&self, id: OccurrenceId) -> bool {
        self.product.grounded_occurrences.contains(&id)
    }

    pub fn grounded_occurrences(&self) -> impl Iterator<Item = OccurrenceId> + '_ {
        self.product.grounded_occurrences.iter().cloned()
    }

    #[must_use]
    pub fn assembly_mate(&self, id: AssemblyMateId) -> Option<&AssemblyMate> {
        self.product.assembly_mates.get(&id).map(Arc::as_ref)
    }

    pub fn assembly_mates(&self) -> impl Iterator<Item = &AssemblyMate> {
        self.product.assembly_mates.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_joint(&self, id: AssemblyJointId) -> Option<&AssemblyJoint> {
        self.product.assembly_joints.get(&id).map(Arc::as_ref)
    }

    pub fn assembly_joints(&self) -> impl Iterator<Item = &AssemblyJoint> {
        self.product.assembly_joints.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_motion_coupling(
        &self,
        id: AssemblyMotionCouplingId,
    ) -> Option<&AssemblyMotionCoupling> {
        self.product
            .assembly_motion_couplings
            .get(&id)
            .map(Arc::as_ref)
    }

    pub fn assembly_motion_couplings(&self) -> impl Iterator<Item = &AssemblyMotionCoupling> {
        self.product
            .assembly_motion_couplings
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_motion_study(&self, id: AssemblyMotionStudyId) -> Option<&AssemblyMotionStudy> {
        self.product
            .assembly_motion_studies
            .get(&id)
            .map(Arc::as_ref)
    }

    pub fn assembly_motion_studies(&self) -> impl Iterator<Item = &AssemblyMotionStudy> {
        self.product
            .assembly_motion_studies
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn drawing_sheet(&self, id: DrawingSheetId) -> Option<&DrawingSheet> {
        self.product.drawing_sheets.get(&id).map(Arc::as_ref)
    }

    pub fn drawing_sheets(&self) -> impl Iterator<Item = &DrawingSheet> {
        self.product.drawing_sheets.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_dof_diagnostic(&self, id: OccurrenceId) -> Option<AssemblyDofDiagnostic> {
        self.product.occurrences.contains_key(&id).then(|| {
            let grounded = self.occurrence_is_grounded(id);
            AssemblyDofDiagnostic {
                occurrence_id: id,
                status: if grounded {
                    AssemblyDofStatus::Grounded
                } else {
                    AssemblyDofStatus::PendingSolve
                },
                remaining_dof: grounded.then_some(0),
                incident_mate_ids: self
                    .product
                    .assembly_mates
                    .values()
                    .filter(|mate| {
                        mate.endpoint_a().occurrence_id() == id
                            || mate.endpoint_b().occurrence_id() == id
                    })
                    .map(|mate| mate.id())
                    .collect(),
            }
        })
    }

    #[must_use]
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.product.groups.get(&id).map(Arc::as_ref)
    }

    pub fn definitions(&self) -> impl Iterator<Item = &Definition> {
        self.product.definitions.values().map(Arc::as_ref)
    }

    pub fn features(&self) -> impl Iterator<Item = &Feature> {
        self.product.features.values().map(Arc::as_ref)
    }

    pub fn occurrences(&self) -> impl Iterator<Item = &Occurrence> {
        self.product.occurrences.values().map(Arc::as_ref)
    }

    pub fn groups(&self) -> impl Iterator<Item = &Group> {
        self.product.groups.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn local_occurrence(&self, key: LocalOccurrenceKey) -> Option<&LocalOccurrence> {
        self.product.local_occurrences.get(&key).map(Arc::as_ref)
    }

    #[must_use]
    pub fn local_group(&self, key: LocalGroupKey) -> Option<&LocalGroup> {
        self.product.local_groups.get(&key).map(Arc::as_ref)
    }

    pub fn local_occurrences(&self) -> impl Iterator<Item = &LocalOccurrence> {
        self.product.local_occurrences.values().map(Arc::as_ref)
    }

    pub fn local_groups(&self) -> impl Iterator<Item = &LocalGroup> {
        self.product.local_groups.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn scene_query(&self) -> Vec<SceneOccurrence> {
        let mut occurrences = Vec::new();
        for occurrence in self.product.occurrences.values() {
            let definition = &self.product.definitions[&occurrence.definition_id];
            let instance_path = InstancePath::root(occurrence.id);
            let world_transform = self
                .world_transform_for_occurrence(occurrence.id)
                .expect("validated occurrence hierarchy has a world transform");
            let visible = self
                .occurrence_effectively_visible(occurrence.id)
                .expect("validated occurrence is queryable");
            occurrences.push(SceneOccurrence {
                occurrence_id: occurrence.id,
                instance_path: instance_path.clone(),
                definition_id: definition.id,
                occurrence_name: occurrence.name.clone(),
                definition_name: definition.name.clone(),
                transform: world_transform,
                parent: occurrence.parent,
                local_parent: None,
                visible,
                shared_occurrence_count: 0,
            });
            project_local_occurrences(
                &self.product,
                occurrence.id,
                definition.id,
                &instance_path,
                world_transform,
                visible,
                &mut occurrences,
            );
        }

        let mut sharing = BTreeMap::<DefinitionId, usize>::new();
        for occurrence in &occurrences {
            *sharing.entry(occurrence.definition_id).or_default() += 1;
        }
        for occurrence in &mut occurrences {
            occurrence.shared_occurrence_count = sharing[&occurrence.definition_id];
        }
        occurrences
    }

    pub fn bind_scene_query(
        &self,
        context: SceneQueryContext,
    ) -> Result<BoundSceneQuery, SceneQueryError> {
        let context_is_valid = match &context {
            SceneQueryContext::Group(group_id) => self.group(*group_id).is_some(),
            SceneQueryContext::Definition {
                definition_id,
                instance_path,
            } => {
                self.resolve_instance_path(instance_path)
                    .is_ok_and(|resolved| resolved.definition_id == *definition_id)
                    && self.scene_query().into_iter().any(|occurrence| {
                        occurrence.instance_path == *instance_path && occurrence.visible
                    })
            }
        };
        if !context_is_valid {
            return Err(SceneQueryError::InvalidContext);
        }
        Ok(BoundSceneQuery {
            document_id: self.document_id(),
            source_revision: self.revision_id(),
            source_digest: self.canonical_digest(),
            context,
        })
    }

    pub fn scene_query_in(
        &self,
        query: &BoundSceneQuery,
    ) -> Result<Vec<SceneOccurrence>, SceneQueryError> {
        if query.document_id != self.document_id()
            || query.source_revision != self.revision_id()
            || query.source_digest != self.canonical_digest()
        {
            return Err(SceneQueryError::SnapshotMismatch);
        }
        let mut occurrences = self.scene_query();
        occurrences.retain(|occurrence| {
            occurrence.visible
                && match &query.context {
                    SceneQueryContext::Group(group_id) => {
                        occurrence.instance_path.is_root()
                            && self
                                .occurrence(occurrence.instance_path.root_occurrence())
                                .is_some_and(|root| root.parent() == Some(*group_id))
                    }
                    SceneQueryContext::Definition {
                        definition_id,
                        instance_path,
                    } => {
                        self.resolve_instance_path(instance_path)
                            .is_ok_and(|resolved| resolved.definition_id == *definition_id)
                            && occurrence.instance_path.root_occurrence()
                                == instance_path.root_occurrence()
                            && occurrence
                                .instance_path
                                .steps()
                                .starts_with(instance_path.steps())
                            && occurrence.instance_path.steps()[instance_path.steps().len()..]
                                .iter()
                                .filter(|step| matches!(step, InstancePathStep::Occurrence(_)))
                                .count()
                                <= 1
                    }
                }
        });
        let mut sharing = BTreeMap::<DefinitionId, usize>::new();
        for occurrence in &occurrences {
            *sharing.entry(occurrence.definition_id).or_default() += 1;
        }
        for occurrence in &mut occurrences {
            occurrence.shared_occurrence_count = sharing[&occurrence.definition_id];
        }
        Ok(occurrences)
    }

    pub fn resolve_instance_path(
        &self,
        path: &InstancePath,
    ) -> Result<ResolvedInstance, CanonicalError> {
        let root = self
            .occurrence(path.root)
            .ok_or(CanonicalError::InvalidInstancePath)?;
        let mut definition_id = root.definition_id;
        let mut transform = self
            .world_transform_for_occurrence(root.id)
            .ok_or(CanonicalError::InvalidInstancePath)?;
        let mut parent = None;
        for step in &path.steps {
            match *step {
                InstancePathStep::Group(local_id) => {
                    let group = self
                        .local_group(LocalGroupKey {
                            definition_id,
                            local_id,
                        })
                        .ok_or(CanonicalError::InvalidInstancePath)?;
                    if group.parent != parent {
                        return Err(CanonicalError::InvalidInstancePath);
                    }
                    transform = transform.compose(group.transform);
                    parent = Some(local_id);
                }
                InstancePathStep::Occurrence(local_id) => {
                    let occurrence = self
                        .local_occurrence(LocalOccurrenceKey {
                            definition_id,
                            local_id,
                        })
                        .ok_or(CanonicalError::InvalidInstancePath)?;
                    if occurrence.parent != parent {
                        return Err(CanonicalError::InvalidInstancePath);
                    }
                    transform = transform.compose(occurrence.transform);
                    definition_id = occurrence.definition_id;
                    parent = None;
                }
            }
        }
        Ok(ResolvedInstance {
            definition_id,
            world_transform: transform,
        })
    }

    #[must_use]
    pub fn world_transform_for_group(&self, id: GroupId) -> Option<Transform> {
        let mut lineage = Vec::new();
        let mut cursor = Some(id);
        while let Some(group_id) = cursor {
            let group = self.group(group_id)?;
            lineage.push(group_id);
            cursor = group.parent;
        }
        lineage.reverse();
        Some(
            lineage
                .into_iter()
                .fold(Transform::identity(), |transform, group_id| {
                    transform.compose(self.product.groups[&group_id].transform)
                }),
        )
    }

    #[must_use]
    pub fn world_transform_for_occurrence(&self, id: OccurrenceId) -> Option<Transform> {
        let occurrence = self.occurrence(id)?;
        let parent_transform = occurrence
            .parent
            .map_or(Some(Transform::identity()), |parent| {
                self.world_transform_for_group(parent)
            })?;
        Some(parent_transform.compose(occurrence.transform))
    }

    pub(crate) fn product(&self) -> &ProductModel {
        &self.product
    }
}

fn project_local_occurrences(
    product: &ProductModel,
    root_occurrence_id: OccurrenceId,
    owner_definition_id: DefinitionId,
    owner_path: &InstancePath,
    owner_transform: Transform,
    owner_visible: bool,
    output: &mut Vec<SceneOccurrence>,
) {
    let definition = &product.definitions[&owner_definition_id];
    for local_id in &definition.local_occurrence_ids {
        let local = &product.local_occurrences[&LocalOccurrenceKey {
            definition_id: owner_definition_id,
            local_id: *local_id,
        }];
        let mut group_lineage = Vec::new();
        let mut parent = local.parent;
        while let Some(local_group_id) = parent {
            let group = &product.local_groups[&LocalGroupKey {
                definition_id: owner_definition_id,
                local_id: local_group_id,
            }];
            group_lineage.push(local_group_id);
            parent = group.parent;
        }
        group_lineage.reverse();

        let mut path = owner_path.clone();
        let mut world_transform = owner_transform;
        for local_group_id in group_lineage {
            let group = &product.local_groups[&LocalGroupKey {
                definition_id: owner_definition_id,
                local_id: local_group_id,
            }];
            path = path.with_step(InstancePathStep::Group(local_group_id));
            world_transform = world_transform.compose(group.transform);
        }
        path = path.with_step(InstancePathStep::Occurrence(*local_id));
        world_transform = world_transform.compose(local.transform);
        let target_definition = &product.definitions[&local.definition_id];
        let tag_visible = local
            .tag
            .and_then(|tag_id| product.tags.get(&tag_id))
            .is_none_or(|tag| tag.visible);
        let visible = owner_visible && local.visible && tag_visible;
        output.push(SceneOccurrence {
            occurrence_id: root_occurrence_id,
            instance_path: path.clone(),
            definition_id: local.definition_id,
            occurrence_name: local.name.clone(),
            definition_name: target_definition.name.clone(),
            transform: world_transform,
            parent: None,
            local_parent: local.parent,
            visible,
            shared_occurrence_count: 0,
        });
        project_local_occurrences(
            product,
            root_occurrence_id,
            local.definition_id,
            &path,
            world_transform,
            visible,
            output,
        );
    }
}

#[derive(Clone)]
pub struct Revision {
    id: u64,
    snapshot: Snapshot,
    batch_digest: String,
    recomputed_nodes: BTreeSet<NodeId>,
    dirty_features: BTreeSet<FeatureId>,
    feature_states: BTreeMap<FeatureId, FeatureEvaluationState>,
    evaluation: Option<EvaluationReport>,
}

impl Revision {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    #[must_use]
    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }

    #[must_use]
    pub const fn recomputed_nodes(&self) -> &BTreeSet<NodeId> {
        &self.recomputed_nodes
    }

    #[must_use]
    pub const fn dirty_features(&self) -> &BTreeSet<FeatureId> {
        &self.dirty_features
    }

    #[must_use]
    pub const fn feature_states(&self) -> &BTreeMap<FeatureId, FeatureEvaluationState> {
        &self.feature_states
    }

    #[must_use]
    pub const fn evaluation(&self) -> Option<&EvaluationReport> {
        self.evaluation.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct DerivedResultKey {
    pub document_id: DocumentId,
    pub revision_id: u64,
    pub root_rule_node_id: NodeId,
    pub slot_path: SlotPath,
    pub input_digest: String,
    pub result_digest: String,
    pub evaluator: String,
    pub backend: Option<String>,
    pub schema: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DerivedResultClassification {
    Current,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactReferenceRebind {
    pub lineage_digest: String,
    pub resolution: ExactReferenceResolution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DerivedResultPayload {
    Evaluation(DerivedResultKey),
    ExactReference(BodySubshapeRef),
    ExactReferenceRebinds(Vec<ExactReferenceRebind>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedResultEvent {
    pub document_id: DocumentId,
    pub revision_id: u64,
    pub canonical_digest: String,
    pub classification: DerivedResultClassification,
    pub payload: DerivedResultPayload,
}

pub enum ExactReferenceEvidence {
    Reference(Box<BodySubshapeRef>),
    Registry(ExactResultRegistry),
}

impl From<BodySubshapeRef> for ExactReferenceEvidence {
    fn from(reference: BodySubshapeRef) -> Self {
        Self::Reference(Box::new(reference))
    }
}

impl From<&ExactResultRegistry> for ExactReferenceEvidence {
    fn from(results: &ExactResultRegistry) -> Self {
        Self::Registry(results.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReferenceEvidenceError {
    InvalidLineage,
    WrongDocument,
    ProducerNotFound,
    ProducerDefinitionMismatch,
}

impl fmt::Display for ReferenceEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineage => formatter.write_str("exact reference lineage is invalid"),
            Self::WrongDocument => {
                formatter.write_str("exact reference belongs to another document")
            }
            Self::ProducerNotFound => formatter.write_str("exact reference producer was not found"),
            Self::ProducerDefinitionMismatch => {
                formatter.write_str("exact reference producer belongs to another definition")
            }
        }
    }
}

impl std::error::Error for ReferenceEvidenceError {}

struct HumanConfirmationPolicy {
    verifying_key: VerifyingKey,
    epoch: u64,
    consumed_signatures: BTreeSet<[u8; 64]>,
}

pub struct DocumentStore {
    revisions: Vec<Arc<Revision>>,
    cursor: usize,
    next_revision_id: u64,
    evaluation_registry: BTreeMap<DerivedResultKey, DerivedResultEvent>,
    human_confirmation_policy: Option<Box<HumanConfirmationPolicy>>,
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentStore {
    #[must_use]
    pub fn new() -> Self {
        Self::from_product(0, ProductModel::default())
            .expect("an empty canonical document is valid")
    }

    pub fn configure_human_confirmation_policy(
        &mut self,
        verifying_key: [u8; 32],
        epoch: u64,
    ) -> Result<(), HumanConfirmationError> {
        if epoch == 0
            || self
                .human_confirmation_policy
                .as_ref()
                .is_some_and(|current| epoch <= current.epoch)
        {
            return Err(HumanConfirmationError::PolicyEpochInvalid);
        }
        let verifying_key = VerifyingKey::from_bytes(&verifying_key)
            .map_err(|_| HumanConfirmationError::InvalidVerifyingKey)?;
        self.human_confirmation_policy = Some(Box::new(HumanConfirmationPolicy {
            verifying_key,
            epoch,
            consumed_signatures: BTreeSet::new(),
        }));
        Ok(())
    }

    pub(crate) fn from_product(
        revision_id: u64,
        product: ProductModel,
    ) -> Result<Self, CanonicalError> {
        validate_graph(&product.evaluator_nodes)?;
        validate_overrides(&product)?;
        validate_product(&product)?;
        let next_revision_id = revision_id
            .checked_add(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let feature_states = FeatureDependencyGraph::from_product(&product)?
            .evaluation_states(&BTreeSet::new(), &BTreeSet::new());
        let snapshot = Snapshot {
            revision_id,
            product: Arc::new(product),
        };
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot,
            batch_digest: String::new(),
            recomputed_nodes: BTreeSet::new(),
            dirty_features: BTreeSet::new(),
            feature_states,
            evaluation: None,
        });
        Ok(Self {
            revisions: vec![revision],
            cursor: 0,
            next_revision_id,
            evaluation_registry: BTreeMap::new(),
            human_confirmation_policy: None,
        })
    }

    #[must_use]
    pub fn current(&self) -> Snapshot {
        self.revisions[self.cursor].snapshot.clone()
    }

    pub fn register_exact_reference_evidence(
        &mut self,
        evidence: impl Into<ExactReferenceEvidence>,
    ) -> Result<(), ReferenceEvidenceError> {
        match evidence.into() {
            ExactReferenceEvidence::Reference(reference) => {
                let current = &self.revisions[self.cursor];
                if !reference.has_valid_lineage() {
                    return Err(ReferenceEvidenceError::InvalidLineage);
                }
                if reference.document_id != current.snapshot.document_id() {
                    return Err(ReferenceEvidenceError::WrongDocument);
                }
                let producer = current
                    .snapshot
                    .feature(reference.producer_feature_id)
                    .ok_or(ReferenceEvidenceError::ProducerNotFound)?;
                if producer.definition_id() != reference.definition_id {
                    return Err(ReferenceEvidenceError::ProducerDefinitionMismatch);
                }
                let matches_request = ExactFeatureChainRequest::from_snapshot_for_producer(
                    &current.snapshot,
                    reference.definition_id,
                    reference.producer_feature_id,
                )
                .is_ok_and(|request| reference.matches_request(&request))
                    || ExactRevolveRequest::from_snapshot(
                        &current.snapshot,
                        reference.definition_id,
                    )
                    .is_ok_and(|request| reference_matches_revolve_request(&reference, &request));
                if !matches_request {
                    return Err(ReferenceEvidenceError::InvalidLineage);
                }
                let event = DerivedResultEvent {
                    document_id: current.snapshot.document_id(),
                    revision_id: current.snapshot.revision_id(),
                    canonical_digest: current.snapshot.canonical_digest(),
                    classification: DerivedResultClassification::Current,
                    payload: DerivedResultPayload::ExactReference(*reference),
                };
                if !self.register_derived_result(event) {
                    return Err(ReferenceEvidenceError::WrongDocument);
                }
            }
            ExactReferenceEvidence::Registry(results) => {
                let current = &self.revisions[self.cursor];
                let rebinds = current
                    .snapshot
                    .features()
                    .filter_map(|feature| match feature.kind() {
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::PlanarFace { reference, .. },
                            ..
                        }) => Some(ExactReferenceRebind {
                            lineage_digest: reference.lineage_digest.clone(),
                            resolution: results.resolve_reference(&current.snapshot, reference),
                        }),
                        _ => None,
                    })
                    .collect();
                let event = DerivedResultEvent {
                    document_id: current.snapshot.document_id(),
                    revision_id: current.snapshot.revision_id(),
                    canonical_digest: current.snapshot.canonical_digest(),
                    classification: DerivedResultClassification::Current,
                    payload: DerivedResultPayload::ExactReferenceRebinds(rebinds),
                };
                if !self.register_derived_result(event) {
                    return Err(ReferenceEvidenceError::InvalidLineage);
                }
            }
        }
        Ok(())
    }

    fn register_derived_result(&mut self, event: DerivedResultEvent) -> bool {
        let current = &self.revisions[self.cursor];
        if event.document_id != current.snapshot.document_id()
            || event.revision_id != current.snapshot.revision_id()
            || event.canonical_digest != current.snapshot.canonical_digest()
        {
            return false;
        }
        match &event.payload {
            DerivedResultPayload::Evaluation(key)
                if key.document_id != event.document_id || key.revision_id != event.revision_id =>
            {
                return false;
            }
            DerivedResultPayload::ExactReference(reference)
                if reference.document_id != event.document_id =>
            {
                return false;
            }
            _ => {}
        }
        match &event.payload {
            DerivedResultPayload::Evaluation(key) => {
                self.evaluation_registry.insert(key.clone(), event);
            }
            DerivedResultPayload::ExactReference(reference) => {
                let anchor_is_current = |support: &BodySubshapeRef| {
                    ExactFeatureChainRequest::from_snapshot_for_producer(
                        &current.snapshot,
                        support.definition_id,
                        support.producer_feature_id,
                    )
                    .is_ok_and(|request| support.matches_request(&request))
                };
                let conflicts_with_anchor = current.snapshot.features().any(|feature| {
                    matches!(
                        feature.kind(),
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::PlanarFace {
                                reference: support,
                                ..
                            },
                            ..
                        }) if support.lineage_digest == reference.lineage_digest
                            && (support.definition_id != reference.definition_id
                                || support.profile_feature_id != reference.profile_feature_id
                                || support.producer_feature_id != reference.producer_feature_id
                                || support.semantic_role != reference.semantic_role
                                || support.source_element_id != reference.source_element_id
                                || support.expected_type != reference.expected_type
                                || (anchor_is_current(support) && support.as_ref() != reference))
                    )
                });
                if conflicts_with_anchor {
                    return false;
                }
                let mut product = current.snapshot.product.as_ref().clone();
                product.exact_reference_evidence.insert(
                    reference.lineage_digest.clone(),
                    Arc::new(reference.clone()),
                );
                if rebind_planar_face_reference(&mut product, reference).is_err() {
                    return false;
                }
                self.revisions[self.cursor] = Arc::new(Revision {
                    id: current.id,
                    snapshot: Snapshot {
                        revision_id: current.snapshot.revision_id,
                        product: Arc::new(product),
                    },
                    batch_digest: current.batch_digest.clone(),
                    recomputed_nodes: current.recomputed_nodes.clone(),
                    dirty_features: current.dirty_features.clone(),
                    feature_states: current.feature_states.clone(),
                    evaluation: current.evaluation.clone(),
                });
            }
            DerivedResultPayload::ExactReferenceRebinds(rebinds) => {
                let mut product = current.snapshot.product.as_ref().clone();
                for rebind in rebinds {
                    match &rebind.resolution {
                        ExactReferenceResolution::Resolved { reference } => {
                            let matches_request =
                                ExactFeatureChainRequest::from_snapshot_for_producer(
                                    &current.snapshot,
                                    reference.definition_id,
                                    reference.producer_feature_id,
                                )
                                .is_ok_and(|request| reference.matches_request(&request));
                            if !matches_request || reference.lineage_digest != rebind.lineage_digest
                            {
                                return false;
                            }
                            product.exact_reference_evidence.insert(
                                rebind.lineage_digest.clone(),
                                Arc::new(reference.as_ref().clone()),
                            );
                            if rebind_planar_face_reference(&mut product, reference).is_err() {
                                return false;
                            }
                        }
                        ExactReferenceResolution::Ambiguous { .. } => {
                            product
                                .exact_reference_evidence
                                .remove(&rebind.lineage_digest);
                            set_planar_face_reference_health(
                                &mut product,
                                &rebind.lineage_digest,
                                WorkplaneSupportHealth::Ambiguous,
                            );
                        }
                        ExactReferenceResolution::Lost => {
                            product
                                .exact_reference_evidence
                                .remove(&rebind.lineage_digest);
                            set_planar_face_reference_health(
                                &mut product,
                                &rebind.lineage_digest,
                                WorkplaneSupportHealth::Lost,
                            );
                        }
                        ExactReferenceResolution::Quarantined { .. } => {
                            product
                                .exact_reference_evidence
                                .remove(&rebind.lineage_digest);
                            set_planar_face_reference_health(
                                &mut product,
                                &rebind.lineage_digest,
                                WorkplaneSupportHealth::Stale,
                            );
                        }
                    }
                }
                if validate_product(&product).is_err() {
                    return false;
                }
                self.revisions[self.cursor] = Arc::new(Revision {
                    id: current.id,
                    snapshot: Snapshot {
                        revision_id: current.snapshot.revision_id,
                        product: Arc::new(product),
                    },
                    batch_digest: current.batch_digest.clone(),
                    recomputed_nodes: current.recomputed_nodes.clone(),
                    dirty_features: current.dirty_features.clone(),
                    feature_states: current.feature_states.clone(),
                    evaluation: current.evaluation.clone(),
                });
            }
        }
        true
    }

    #[must_use]
    pub fn revision_count(&self) -> usize {
        self.revisions.len()
    }

    #[must_use]
    pub const fn visible_undo_steps(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn visible_redo_steps(&self) -> usize {
        self.revisions.len() - self.cursor - 1
    }

    pub fn discard_history_before_current(&mut self) {
        let current = Arc::clone(&self.revisions[self.cursor]);
        self.revisions.clear();
        self.revisions.push(current);
        self.cursor = 0;
    }

    pub fn validate_batch(&self, batch: &CommandBatch) -> Result<(), CanonicalError> {
        self.preview_batch(batch).map(|_| ())
    }

    pub fn preview_batch(&self, batch: &CommandBatch) -> Result<Snapshot, CanonicalError> {
        let snapshot = self.current();
        let mut candidate =
            Self::from_product(snapshot.revision_id(), snapshot.product.as_ref().clone())?;
        candidate.apply_batch(batch)?;
        Ok(candidate.current())
    }

    pub fn apply_batch(&mut self, batch: &CommandBatch) -> Result<Arc<Revision>, CanonicalError> {
        if batch.schema != COMMAND_SCHEMA_V1 {
            return Err(CanonicalError::UnsupportedCommandSchema);
        }
        if batch.commands.is_empty() {
            return Err(CanonicalError::EmptyCommandBatch);
        }

        let current = self.current();
        let mut product = current.product.as_ref().clone();
        let anchored_reference_lineages = product
            .features
            .values()
            .filter_map(|feature| match &feature.kind {
                FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace { reference, .. },
                    ..
                }) => Some(reference.lineage_digest.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        product
            .exact_reference_evidence
            .retain(|lineage, _| anchored_reference_lineages.contains(lineage));
        let mut changed_evaluator_nodes = BTreeSet::new();
        let mut explicit_dirty_features = BTreeSet::new();
        let mut evaluation_identity = EvaluationIdentity::default();
        let mut previous_evaluation = self.revisions[self.cursor].evaluation.clone();

        for command in &batch.commands {
            match command {
                CanonicalCommand::CreateEvaluatorNode {
                    id,
                    name,
                    dimension,
                    dependencies,
                } => {
                    if product.evaluator_nodes.contains_key(id) {
                        return Err(CanonicalError::NodeAlreadyExists(*id));
                    }
                    for dependency in dependencies {
                        if !product.evaluator_nodes.contains_key(dependency) {
                            return Err(CanonicalError::MissingDependency(*dependency));
                        }
                    }
                    let node = EvaluatorNode::parameter(
                        *id,
                        name.clone(),
                        dimension.clone(),
                        dependencies.clone(),
                    )
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(node));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::SetEvaluatorDimension { id, dimension } => {
                    let existing = product
                        .evaluator_nodes
                        .get(id)
                        .ok_or(CanonicalError::NodeNotFound(*id))?;
                    let replacement = EvaluatorNode::parameter(
                        *id,
                        existing.name.clone(),
                        dimension.clone(),
                        existing.dependencies.clone(),
                    )
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(replacement));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::RenameEvaluatorNode { id, name } => {
                    let existing = product
                        .evaluator_nodes
                        .get(id)
                        .ok_or(CanonicalError::NodeNotFound(*id))?;
                    let replacement = match &existing.kind {
                        EvaluatorNodeKind::Parameter { value } => EvaluatorNode::parameter(
                            *id,
                            name.clone(),
                            value.clone(),
                            existing.dependencies.clone(),
                        ),
                        EvaluatorNodeKind::Expression { source, .. } => {
                            EvaluatorNode::expression(*id, name.clone(), source.clone())
                        }
                        EvaluatorNodeKind::Rule {
                            source, outputs, ..
                        } => EvaluatorNode::rule(
                            *id,
                            name.clone(),
                            source.clone(),
                            existing.input_ports.clone(),
                            existing.output_ports.clone(),
                            outputs.clone(),
                            existing.allowed_parameters().to_vec(),
                        ),
                    }
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(replacement));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::CreateExpressionNode {
                    id,
                    name,
                    expression,
                } => {
                    if product.evaluator_nodes.contains_key(id) {
                        return Err(CanonicalError::NodeAlreadyExists(*id));
                    }
                    let node = EvaluatorNode::expression(*id, name.clone(), expression.clone())
                        .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(node));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::CreateRuleNode {
                    id,
                    name,
                    expression,
                    input_ports,
                    output_ports,
                    outputs,
                    override_parameters,
                } => {
                    if product.evaluator_nodes.contains_key(id) {
                        return Err(CanonicalError::NodeAlreadyExists(*id));
                    }
                    let node = EvaluatorNode::rule(
                        *id,
                        name.clone(),
                        expression.clone(),
                        input_ports.clone(),
                        output_ports.clone(),
                        outputs.clone(),
                        override_parameters.clone(),
                    )
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(node));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::SetNodeExpression { id, expression } => {
                    let existing = product
                        .evaluator_nodes
                        .get(id)
                        .ok_or(CanonicalError::NodeNotFound(*id))?;
                    let replacement = match &existing.kind {
                        EvaluatorNodeKind::Expression { .. } => EvaluatorNode::expression(
                            *id,
                            existing.name.clone(),
                            expression.clone(),
                        ),
                        EvaluatorNodeKind::Rule { outputs, .. } => EvaluatorNode::rule(
                            *id,
                            existing.name.clone(),
                            expression.clone(),
                            existing.input_ports.clone(),
                            existing.output_ports.clone(),
                            outputs.clone(),
                            existing.allowed_parameters().to_vec(),
                        ),
                        EvaluatorNodeKind::Parameter { .. } => {
                            return Err(CanonicalError::WrongNodeKind(*id));
                        }
                    }
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(replacement));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::SetRuleOutputs { id, outputs } => {
                    let existing = product
                        .evaluator_nodes
                        .get(id)
                        .ok_or(CanonicalError::NodeNotFound(*id))?;
                    let EvaluatorNodeKind::Rule { source, .. } = &existing.kind else {
                        return Err(CanonicalError::WrongNodeKind(*id));
                    };
                    let replacement = EvaluatorNode::rule(
                        *id,
                        existing.name.clone(),
                        source.clone(),
                        existing.input_ports.clone(),
                        existing.output_ports.clone(),
                        outputs.clone(),
                        existing.allowed_parameters().to_vec(),
                    )
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(replacement));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::UpsertOverride(spec) => {
                    let mut canonical = spec.clone();
                    canonical.health =
                        resolve_derived_identity(&product.evaluator_nodes, &canonical.target);
                    product.overrides.insert(canonical.id, Arc::new(canonical));
                }
                CanonicalCommand::DeleteOverride { id } => {
                    if product.overrides.remove(id).is_none() {
                        return Err(CanonicalError::OverrideNotFound(*id));
                    }
                }
                CanonicalCommand::UpsertFeatureParameterBinding(binding) => {
                    product.feature_parameter_provenance.remove(&binding.target);
                    product
                        .feature_parameter_bindings
                        .insert(binding.target.clone(), Arc::new(binding.clone()));
                }
                CanonicalCommand::DeleteFeatureParameterBinding { target } => {
                    if product.feature_parameter_bindings.remove(target).is_none() {
                        return Err(CanonicalError::FeatureParameterBindingNotFound(
                            target.clone(),
                        ));
                    }
                    product.feature_parameter_provenance.remove(target);
                }
                CanonicalCommand::RecomputeFeatureParameters { identity } => {
                    let affected = if changed_evaluator_nodes.is_empty() {
                        None
                    } else {
                        Some(dependent_closure(
                            &product.evaluator_nodes,
                            &changed_evaluator_nodes,
                        ))
                    };
                    let report = recompute_feature_parameters(
                        &mut product,
                        identity,
                        affected.as_ref(),
                        previous_evaluation.as_ref(),
                    )?;
                    previous_evaluation = Some(report.clone());
                    if affected.is_none() {
                        changed_evaluator_nodes.extend(
                            product
                                .feature_parameter_bindings
                                .values()
                                .map(|binding| binding.derived_from.root_rule_node_id),
                        );
                    }
                    evaluation_identity = report.identity;
                }
                CanonicalCommand::UpsertJoint(joint) => {
                    product.joints.insert(joint.id(), Arc::new(joint.clone()));
                }
                CanonicalCommand::DeleteJoint { id } => {
                    if product.joints.remove(id).is_none() {
                        return Err(CanonicalError::JointNotFound(*id));
                    }
                }
                CanonicalCommand::UpsertSpace(space) => {
                    product.spaces.insert(space.id(), Arc::new(space.clone()));
                }
                CanonicalCommand::DeleteSpace { id } => {
                    if product.spaces.remove(id).is_none() {
                        return Err(CanonicalError::SpaceNotFound(*id));
                    }
                }
                CanonicalCommand::UpsertClearanceVolume(clearance) => {
                    if clearance.derived_from().is_some_and(|identity| {
                        resolve_derived_identity(&product.evaluator_nodes, identity)
                            != SlotResolution::Resolved
                    }) {
                        return Err(CanonicalError::UnresolvedDerivedOutput);
                    }
                    product
                        .clearance_volumes
                        .insert(clearance.id(), Arc::new(clearance.clone()));
                }
                CanonicalCommand::DeleteClearanceVolume { id } => {
                    if product.clearance_volumes.remove(id).is_none() {
                        return Err(CanonicalError::ClearanceVolumeNotFound(*id));
                    }
                }
                CanonicalCommand::UpsertPersistentDimension(dimension) => {
                    validate_persistent_dimension(dimension)?;
                    product
                        .persistent_dimensions
                        .insert(dimension.id, Arc::new(dimension.clone()));
                }
                CanonicalCommand::DeletePersistentDimension { id } => {
                    if product.persistent_dimensions.remove(id).is_none() {
                        return Err(CanonicalError::PersistentDimensionNotFound(*id));
                    }
                }
                CanonicalCommand::CreateTag { id, name, visible } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    if product.tags.contains_key(id) {
                        return Err(CanonicalError::TagAlreadyExists(*id));
                    }
                    product.tags.insert(
                        *id,
                        Arc::new(Tag {
                            id: *id,
                            name: name.clone(),
                            visible: *visible,
                        }),
                    );
                }
                CanonicalCommand::DeleteTag { id } => {
                    if product
                        .occurrences
                        .values()
                        .any(|occurrence| occurrence.tag == Some(*id))
                        || product
                            .local_occurrences
                            .values()
                            .any(|occurrence| occurrence.tag == Some(*id))
                    {
                        return Err(CanonicalError::TagInUse(*id));
                    }
                    product
                        .tags
                        .remove(id)
                        .ok_or(CanonicalError::TagNotFound(*id))?;
                }
                CanonicalCommand::SetTagVisibility { id, visible } => {
                    let existing = product
                        .tags
                        .get(id)
                        .ok_or(CanonicalError::TagNotFound(*id))?;
                    product.tags.insert(
                        *id,
                        Arc::new(Tag {
                            visible: *visible,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::SetTagName { id, name } => {
                    ensure_name(name)?;
                    let existing = product
                        .tags
                        .get(id)
                        .ok_or(CanonicalError::TagNotFound(*id))?;
                    product.tags.insert(
                        *id,
                        Arc::new(Tag {
                            name: name.clone(),
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::UpsertClassificationDimension {
                    id,
                    name,
                    categories,
                } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    if categories.is_empty()
                        || categories.windows(2).any(|pair| pair[0].0 >= pair[1].0)
                    {
                        return Err(CanonicalError::InvalidClassificationDimension(*id));
                    }
                    let mut canonical_categories = BTreeMap::new();
                    let mut names = BTreeSet::new();
                    for (category_id, category_name) in categories {
                        ensure_product_id(category_id.0)?;
                        ensure_name(category_name)?;
                        if !names.insert(category_name.clone()) {
                            return Err(CanonicalError::InvalidClassificationDimension(*id));
                        }
                        canonical_categories.insert(
                            *category_id,
                            ClassificationCategory {
                                id: *category_id,
                                name: category_name.clone(),
                            },
                        );
                    }
                    if product.classification_assignments.iter().any(
                        |((_, dimension_id), category_id)| {
                            dimension_id == id && !canonical_categories.contains_key(category_id)
                        },
                    ) {
                        return Err(CanonicalError::ClassificationCategoryInUse(*id));
                    }
                    product.classification_dimensions.insert(
                        *id,
                        Arc::new(ClassificationDimension {
                            id: *id,
                            name: name.clone(),
                            categories: canonical_categories,
                        }),
                    );
                }
                CanonicalCommand::SetOccurrenceClassification {
                    occurrence_id,
                    dimension_id,
                    category_id,
                } => {
                    if !product.occurrences.contains_key(occurrence_id) {
                        return Err(CanonicalError::OccurrenceNotFound(*occurrence_id));
                    }
                    let dimension = product.classification_dimensions.get(dimension_id).ok_or(
                        CanonicalError::ClassificationDimensionNotFound(*dimension_id),
                    )?;
                    if let Some(category_id) = category_id {
                        if !dimension.categories.contains_key(category_id) {
                            return Err(CanonicalError::ClassificationCategoryNotFound(
                                *dimension_id,
                                *category_id,
                            ));
                        }
                        product
                            .classification_assignments
                            .insert((*occurrence_id, *dimension_id), *category_id);
                    } else {
                        product
                            .classification_assignments
                            .remove(&(*occurrence_id, *dimension_id));
                    }
                }
                CanonicalCommand::CreateCollection { id, name } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    if product.collections.contains_key(id) {
                        return Err(CanonicalError::CollectionAlreadyExists(*id));
                    }
                    product.collections.insert(
                        *id,
                        Arc::new(Collection {
                            id: *id,
                            name: name.clone(),
                            occurrence_ids: BTreeSet::new(),
                        }),
                    );
                }
                CanonicalCommand::DeleteCollection { id } => {
                    product
                        .collections
                        .remove(id)
                        .ok_or(CanonicalError::CollectionNotFound(*id))?;
                }
                CanonicalCommand::SetCollectionOccurrences { id, occurrence_ids } => {
                    let existing = product
                        .collections
                        .get(id)
                        .ok_or(CanonicalError::CollectionNotFound(*id))?;
                    if occurrence_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                        return Err(CanonicalError::CollectionMembershipNotCanonical(*id));
                    }
                    let canonical = occurrence_ids.iter().cloned().collect::<BTreeSet<_>>();
                    for occurrence_id in &canonical {
                        if !product.occurrences.contains_key(occurrence_id) {
                            return Err(CanonicalError::OccurrenceNotFound(*occurrence_id));
                        }
                    }
                    product.collections.insert(
                        *id,
                        Arc::new(Collection {
                            occurrence_ids: canonical,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::RecordImport(receipt) => {
                    receipt
                        .validate()
                        .map_err(|_| CanonicalError::InvalidImportReceipt)?;
                    if product.import_receipts.contains_key(&receipt.id()) {
                        return Err(CanonicalError::ImportAlreadyExists(receipt.id()));
                    }
                    for output in receipt.outputs() {
                        let exists = match output {
                            ImportOutputRef::Definition(id) => product.definitions.contains_key(id),
                            ImportOutputRef::Feature(id) => product.features.contains_key(id),
                            ImportOutputRef::Occurrence(id) => product.occurrences.contains_key(id),
                        };
                        if !exists {
                            return Err(CanonicalError::InvalidImportReceipt);
                        }
                    }
                    product
                        .import_receipts
                        .insert(receipt.id(), Arc::new(receipt.clone()));
                }
                CanonicalCommand::CreateDefinition { id, name } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    if product.definitions.contains_key(id) {
                        return Err(CanonicalError::DefinitionAlreadyExists(*id));
                    }
                    product
                        .definitions
                        .insert(*id, Arc::new(new_definition(*id, name.clone())));
                }
                CanonicalCommand::DeleteDefinition { id } => {
                    if product
                        .occurrences
                        .values()
                        .any(|occurrence| occurrence.definition_id == *id)
                    {
                        return Err(CanonicalError::DefinitionInUse(*id));
                    }
                    let definition = product
                        .definitions
                        .remove(id)
                        .ok_or(CanonicalError::DefinitionNotFound(*id))?;
                    product
                        .body_feature_suppression
                        .retain(|(definition_id, _), _| definition_id != id);
                    for feature_id in &definition.feature_ids {
                        product.features.remove(feature_id);
                        product
                            .feature_parameter_bindings
                            .retain(|target, _| target.feature_id != *feature_id);
                        product
                            .feature_parameter_provenance
                            .retain(|target, _| target.feature_id != *feature_id);
                    }
                }
                CanonicalCommand::RenameDefinition { id, name } => {
                    ensure_name(name)?;
                    let existing = product
                        .definitions
                        .get(id)
                        .ok_or(CanonicalError::DefinitionNotFound(*id))?;
                    product.definitions.insert(
                        *id,
                        Arc::new(Definition {
                            name: name.clone(),
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::CreateBody {
                    definition_id,
                    id,
                    name,
                    visible,
                } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    let definition = product
                        .definitions
                        .get(definition_id)
                        .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
                    if definition.bodies.contains_key(id) {
                        return Err(CanonicalError::BodyAlreadyExists(*definition_id, *id));
                    }
                    let mut replacement = definition.as_ref().clone();
                    replacement.bodies.insert(
                        *id,
                        Body {
                            id: *id,
                            name: name.clone(),
                            visible: *visible,
                            consumed_by: None,
                        },
                    );
                    product
                        .definitions
                        .insert(*definition_id, Arc::new(replacement));
                }
                CanonicalCommand::DeleteBody { definition_id, id } => {
                    let definition = product
                        .definitions
                        .get(definition_id)
                        .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
                    if !definition.bodies.contains_key(id) {
                        return Err(CanonicalError::BodyNotFound(*definition_id, *id));
                    }
                    if definition.active_body_id == *id {
                        return Err(CanonicalError::BodyIsActive(*definition_id, *id));
                    }
                    if definition.feature_body_ownership.values().any(|ownership| {
                        ownership.output_body_id == Some(*id)
                            || ownership.input_body_ids.contains(id)
                    }) {
                        return Err(CanonicalError::BodyInUse(*definition_id, *id));
                    }
                    let mut replacement = definition.as_ref().clone();
                    replacement.bodies.remove(id);
                    product
                        .definitions
                        .insert(*definition_id, Arc::new(replacement));
                }
                CanonicalCommand::RenameBody {
                    definition_id,
                    id,
                    name,
                } => {
                    ensure_name(name)?;
                    let definition = product
                        .definitions
                        .get(definition_id)
                        .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
                    let body = definition
                        .bodies
                        .get(id)
                        .ok_or(CanonicalError::BodyNotFound(*definition_id, *id))?;
                    let mut replacement = definition.as_ref().clone();
                    replacement.bodies.insert(
                        *id,
                        Body {
                            name: name.clone(),
                            ..body.clone()
                        },
                    );
                    product
                        .definitions
                        .insert(*definition_id, Arc::new(replacement));
                }
                CanonicalCommand::SetActiveBody { definition_id, id } => {
                    let definition = product
                        .definitions
                        .get(definition_id)
                        .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
                    let body = definition
                        .bodies
                        .get(id)
                        .ok_or(CanonicalError::BodyNotFound(*definition_id, *id))?;
                    if body.consumed_by.is_some() {
                        return Err(CanonicalError::InvalidBodyAuthoringPlan);
                    }
                    let mut replacement = definition.as_ref().clone();
                    replacement.active_body_id = *id;
                    product
                        .definitions
                        .insert(*definition_id, Arc::new(replacement));
                }
                CanonicalCommand::SetBodyVisibility {
                    definition_id,
                    id,
                    visible,
                } => {
                    let definition = product
                        .definitions
                        .get(definition_id)
                        .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
                    let body = definition
                        .bodies
                        .get(id)
                        .ok_or(CanonicalError::BodyNotFound(*definition_id, *id))?;
                    let mut replacement = definition.as_ref().clone();
                    replacement.bodies.insert(
                        *id,
                        Body {
                            visible: *visible,
                            ..body.clone()
                        },
                    );
                    product
                        .definitions
                        .insert(*definition_id, Arc::new(replacement));
                }
                CanonicalCommand::ConsumeBody {
                    definition_id,
                    id,
                    by_feature_id,
                } => {
                    let definition = product
                        .definitions
                        .get(definition_id)
                        .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
                    let body = definition
                        .bodies
                        .get(id)
                        .ok_or(CanonicalError::BodyNotFound(*definition_id, *id))?;
                    let feature = product
                        .features
                        .get(by_feature_id)
                        .ok_or(CanonicalError::FeatureNotFound(*by_feature_id))?;
                    let ownership = definition
                        .feature_body_ownership
                        .get(by_feature_id)
                        .ok_or(CanonicalError::InvalidBodyAuthoringPlan)?;
                    if body.consumed_by.is_some()
                        || definition.active_body_id == *id
                        || feature.definition_id != *definition_id
                        || !matches!(feature.kind, FeatureKind::Boolean { .. })
                        || !ownership.input_body_ids.contains(id)
                        || ownership.output_body_id == Some(*id)
                    {
                        return Err(CanonicalError::InvalidBodyAuthoringPlan);
                    }
                    let mut replacement = definition.as_ref().clone();
                    replacement.bodies.insert(
                        *id,
                        Body {
                            consumed_by: Some(*by_feature_id),
                            ..body.clone()
                        },
                    );
                    product
                        .definitions
                        .insert(*definition_id, Arc::new(replacement));
                }
                CanonicalCommand::SetFeatureBodyOwnership { id, ownership } => {
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    validate_feature_body_ownership_change(&product, feature, ownership)?;
                    let definition = &product.definitions[&feature.definition_id];
                    let mut replacement = definition.as_ref().clone();
                    replacement
                        .feature_body_ownership
                        .insert(*id, ownership.clone());
                    product
                        .definitions
                        .insert(feature.definition_id, Arc::new(replacement));
                }
                CanonicalCommand::SetBodyFeatureSuppression {
                    definition_id,
                    body_id,
                    suppressed_feature_ids,
                } => {
                    let graph = FeatureDependencyGraph::from_product(&product)?;
                    validate_body_feature_suppression(
                        &product,
                        *definition_id,
                        *body_id,
                        suppressed_feature_ids,
                        &graph,
                    )?;
                    let key = (*definition_id, *body_id);
                    let current_suppressed = product
                        .body_feature_suppression
                        .get(&key)
                        .cloned()
                        .unwrap_or_default();
                    let requested_suppressed = suppressed_feature_ids
                        .iter()
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    if current_suppressed == requested_suppressed {
                        return Err(CanonicalError::FeatureSuppressionUnchanged(
                            *definition_id,
                            *body_id,
                        ));
                    }
                    explicit_dirty_features.extend(
                        current_suppressed
                            .iter()
                            .chain(&requested_suppressed)
                            .cloned(),
                    );
                    if requested_suppressed.is_empty() {
                        product.body_feature_suppression.remove(&key);
                    } else {
                        product
                            .body_feature_suppression
                            .insert(key, requested_suppressed);
                    }
                }
                CanonicalCommand::CreateFeature {
                    id,
                    definition_id,
                    name,
                    kind,
                } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    validate_feature_kind(kind)?;
                    validate_topological_feature_context(
                        current.document_id(),
                        *definition_id,
                        kind,
                    )?;
                    if let FeatureKind::Workplane(WorkplaneSpec {
                        support: WorkplaneSupport::PlanarFace { reference, .. },
                        ..
                    }) = kind
                    {
                        let evidence = current
                            .exact_reference_by_lineage(&reference.lineage_digest)
                            .filter(|evidence| *evidence == reference.as_ref())
                            .ok_or(CanonicalError::Sketch(
                                SketchError::InvalidPlanarFaceSupport,
                            ))?;
                        product
                            .exact_reference_evidence
                            .insert(reference.lineage_digest.clone(), Arc::new(evidence.clone()));
                    }
                    if product.features.contains_key(id) {
                        return Err(CanonicalError::FeatureAlreadyExists(*id));
                    }
                    let definition = product
                        .definitions
                        .get(definition_id)
                        .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
                    let ownership = inferred_feature_body_ownership(&product, definition, kind)?;
                    let mut replacement = definition.as_ref().clone();
                    replacement.feature_ids.push(*id);
                    replacement.feature_body_ownership.insert(*id, ownership);
                    product
                        .definitions
                        .insert(*definition_id, Arc::new(replacement));
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: *definition_id,
                            name: name.clone(),
                            kind: kind.clone(),
                        }),
                    );
                }
                CanonicalCommand::DeleteFeature { id } => {
                    let feature = product
                        .features
                        .remove(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    let definition = &product.definitions[&feature.definition_id];
                    let feature_ids = definition
                        .feature_ids
                        .iter()
                        .cloned()
                        .filter(|candidate| candidate != id)
                        .collect();
                    let mut feature_body_ownership = definition.feature_body_ownership.clone();
                    feature_body_ownership.remove(id);
                    for suppressed in product.body_feature_suppression.values_mut() {
                        suppressed.remove(id);
                    }
                    product
                        .body_feature_suppression
                        .retain(|_, suppressed| !suppressed.is_empty());
                    product
                        .feature_parameter_bindings
                        .retain(|target, _| target.feature_id != *id);
                    product
                        .feature_parameter_provenance
                        .retain(|target, _| target.feature_id != *id);
                    product.definitions.insert(
                        feature.definition_id,
                        Arc::new(Definition {
                            feature_ids,
                            feature_body_ownership,
                            ..definition.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::SetFeatureDimension { id, dimension } => {
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    let kind = match feature.kind {
                        FeatureKind::Extrusion { profile, .. } => FeatureKind::Extrusion {
                            profile,
                            height: dimension.clone(),
                        },
                        FeatureKind::Pad(ref spec) => {
                            let mut updated = spec.clone();
                            updated.extent = crate::sketch::FeatureExtent::Blind(dimension.clone());
                            FeatureKind::Pad(updated)
                        }
                        FeatureKind::SketchPocket(ref spec) => {
                            let mut updated = spec.clone();
                            updated.extent = crate::sketch::FeatureExtent::Blind(dimension.clone());
                            FeatureKind::SketchPocket(updated)
                        }
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::Offset { base, .. },
                            ..
                        }) => {
                            let base_frame = product
                                .features
                                .get(&base)
                                .and_then(|feature| match &feature.kind {
                                    FeatureKind::Workplane(spec) => Some(spec.frame),
                                    _ => None,
                                })
                                .ok_or(CanonicalError::Sketch(
                                    SketchError::MissingWorkplaneSupport(base),
                                ))?;
                            FeatureKind::Workplane(WorkplaneSpec {
                                support: WorkplaneSupport::Offset {
                                    base,
                                    distance: dimension.clone(),
                                },
                                frame: base_frame.offset(dimension.millimetres()),
                            })
                        }
                        FeatureKind::Shell {
                            target,
                            ref removed_faces,
                            ..
                        } => FeatureKind::Shell {
                            target,
                            removed_faces: removed_faces.clone(),
                            thickness: dimension.clone(),
                        },
                        FeatureKind::BottleEdgeFinish {
                            target,
                            ref edges,
                            kind,
                            ..
                        } => FeatureKind::BottleEdgeFinish {
                            target,
                            edges: edges.clone(),
                            kind,
                            amount: dimension.clone(),
                        },
                        FeatureKind::Pocket {
                            target, profile, ..
                        } => FeatureKind::Pocket {
                            target,
                            profile,
                            depth: dimension.clone(),
                        },
                        FeatureKind::PlanarOffset { profile, .. } => FeatureKind::PlanarOffset {
                            profile,
                            distance: dimension.clone(),
                        },
                        _ => return Err(CanonicalError::FeatureHasNoDimension(*id)),
                    };
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: feature.definition_id,
                            name: feature.name.clone(),
                            kind,
                        }),
                    );
                }
                CanonicalCommand::SetSketchConstraintDimension {
                    id,
                    constraint_id,
                    dimension,
                } => {
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    let FeatureKind::Sketch(spec) = &feature.kind else {
                        return Err(CanonicalError::FeatureHasNoDimension(*id));
                    };
                    let mut updated = spec.clone();
                    let constraint = updated
                        .constraints
                        .iter_mut()
                        .find(|constraint| constraint.id == *constraint_id)
                        .ok_or(CanonicalError::Sketch(
                            SketchError::InvalidConstraintReference(*constraint_id),
                        ))?;
                    match &mut constraint.kind {
                        SketchConstraintKind::Distance { value, .. }
                        | SketchConstraintKind::Radius { value, .. } => {
                            *value = dimension.clone();
                        }
                        _ => return Err(CanonicalError::FeatureHasNoDimension(*id)),
                    }
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: feature.definition_id,
                            name: feature.name.clone(),
                            kind: FeatureKind::Sketch(updated),
                        }),
                    );
                }
                CanonicalCommand::TranslateProfile { id, delta_mm } => {
                    if !delta_mm.iter().all(|value| value.is_finite())
                        || (delta_mm[0] == 0.0 && delta_mm[1] == 0.0)
                    {
                        return Err(CanonicalError::InvalidProfile);
                    }
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    let translate = |point: &mut [f64; 2]| {
                        point[0] += delta_mm[0];
                        point[1] += delta_mm[1];
                    };
                    let mut kind = feature.kind.clone();
                    match &mut kind {
                        FeatureKind::Profile { points_mm } => {
                            points_mm.iter_mut().for_each(translate);
                        }
                        FeatureKind::SegmentProfile { segments, .. } => {
                            for segment in segments {
                                match segment {
                                    ProfileSegment::Line { start_mm, end_mm } => {
                                        translate(start_mm);
                                        translate(end_mm);
                                    }
                                    ProfileSegment::CircularArc {
                                        start_mm,
                                        end_mm,
                                        center_mm,
                                        ..
                                    } => {
                                        translate(start_mm);
                                        translate(end_mm);
                                        translate(center_mm);
                                    }
                                }
                            }
                        }
                        FeatureKind::SplineProfile { control_points_mm } => {
                            control_points_mm.iter_mut().for_each(translate);
                        }
                        FeatureKind::Sketch(spec) => {
                            for entity in &mut spec.entities {
                                match entity {
                                    SketchEntity::Line {
                                        start_mm, end_mm, ..
                                    } => {
                                        translate(start_mm);
                                        translate(end_mm);
                                    }
                                    SketchEntity::Arc {
                                        start_mm,
                                        end_mm,
                                        center_mm,
                                        ..
                                    } => {
                                        translate(start_mm);
                                        translate(end_mm);
                                        translate(center_mm);
                                    }
                                    SketchEntity::Circle { center_mm, .. } => translate(center_mm),
                                }
                            }
                            for constraint in &mut spec.constraints {
                                if let SketchConstraintKind::FixedPoint { position_mm, .. } =
                                    &mut constraint.kind
                                {
                                    translate(position_mm);
                                }
                            }
                        }
                        _ => return Err(CanonicalError::FeatureIsNotProfile(*id)),
                    }
                    validate_feature_kind(&kind)?;
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: feature.definition_id,
                            name: feature.name.clone(),
                            kind,
                        }),
                    );
                }
                CanonicalCommand::SetBottleControlDimension {
                    id,
                    control,
                    dimension,
                } => {
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    let FeatureKind::BottleProfileControl {
                        profile,
                        body_radius,
                        body_height,
                        shoulder_rise,
                    } = &feature.kind
                    else {
                        return Err(CanonicalError::FeatureHasNoDimension(*id));
                    };
                    let kind = FeatureKind::BottleProfileControl {
                        profile: *profile,
                        body_radius: if *control == BottleControlDimension::BodyRadius {
                            dimension.clone()
                        } else {
                            body_radius.clone()
                        },
                        body_height: if *control == BottleControlDimension::BodyHeight {
                            dimension.clone()
                        } else {
                            body_height.clone()
                        },
                        shoulder_rise: if *control == BottleControlDimension::ShoulderRise {
                            dimension.clone()
                        } else {
                            shoulder_rise.clone()
                        },
                    };
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: feature.definition_id,
                            name: feature.name.clone(),
                            kind,
                        }),
                    );
                }
                CanonicalCommand::SetBottleEdgeFinishKind { id, kind } => {
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    let FeatureKind::BottleEdgeFinish {
                        target,
                        edges,
                        amount,
                        ..
                    } = &feature.kind
                    else {
                        return Err(CanonicalError::FeatureHasNoDimension(*id));
                    };
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: feature.definition_id,
                            name: feature.name.clone(),
                            kind: FeatureKind::BottleEdgeFinish {
                                target: *target,
                                edges: edges.clone(),
                                kind: *kind,
                                amount: amount.clone(),
                            },
                        }),
                    );
                }
                CanonicalCommand::SetProfilePoints { id, points_mm } => {
                    validate_feature_kind(&FeatureKind::Profile {
                        points_mm: points_mm.clone(),
                    })?;
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    if !matches!(feature.kind, FeatureKind::Profile { .. }) {
                        return Err(CanonicalError::FeatureIsNotProfile(*id));
                    }
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: feature.definition_id,
                            name: feature.name.clone(),
                            kind: FeatureKind::Profile {
                                points_mm: points_mm.clone(),
                            },
                        }),
                    );
                }
                CanonicalCommand::CreateOccurrence {
                    id,
                    definition_id,
                    name,
                    transform,
                    parent,
                    tag,
                    visible,
                } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    validate_transform(*transform)?;
                    if product.occurrences.contains_key(id) {
                        return Err(CanonicalError::OccurrenceAlreadyExists(*id));
                    }
                    if let Some(tag_id) = tag
                        && !product.tags.contains_key(tag_id)
                    {
                        return Err(CanonicalError::TagNotFound(*tag_id));
                    }
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            id: *id,
                            definition_id: *definition_id,
                            name: name.clone(),
                            transform: *transform,
                            parent: *parent,
                            tag: *tag,
                            visible: *visible,
                        }),
                    );
                }
                CanonicalCommand::DeleteOccurrence { id } => {
                    if product
                        .collections
                        .values()
                        .any(|collection| collection.occurrence_ids.contains(id))
                    {
                        return Err(CanonicalError::OccurrenceInCollection(*id));
                    }
                    if product.assembly_mates.values().any(|mate| {
                        mate.endpoint_a().occurrence_id() == *id
                            || mate.endpoint_b().occurrence_id() == *id
                    }) {
                        return Err(CanonicalError::OccurrenceInAssemblyMate(*id));
                    }
                    if product.assembly_joints.values().any(|joint| {
                        joint.parent_occurrence_id() == *id || joint.child_occurrence_id() == *id
                    }) {
                        return Err(CanonicalError::OccurrenceInAssemblyJoint(*id));
                    }
                    product
                        .occurrences
                        .remove(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.grounded_occurrences.remove(id);
                    product
                        .classification_assignments
                        .retain(|(occurrence_id, _), _| occurrence_id != id);
                }
                CanonicalCommand::SetOccurrenceTransform { id, transform } => {
                    validate_transform(*transform)?;
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            transform: *transform,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::RenameEntity { id, name } => {
                    ensure_name(name)?;
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            name: name.clone(),
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::GuardAssemblyRecompute {
                    source_revision,
                    source_digest,
                } => {
                    if current.revision_id() != *source_revision
                        || current.canonical_digest() != *source_digest
                    {
                        return Err(CanonicalError::StaleAssemblySolve);
                    }
                }
                CanonicalCommand::ApplyAssemblySolve {
                    source_revision,
                    source_digest,
                    transforms,
                } => {
                    if current.revision_id() != *source_revision
                        || current.canonical_digest() != *source_digest
                    {
                        return Err(CanonicalError::StaleAssemblySolve);
                    }
                    if transforms.is_empty()
                        || transforms.windows(2).any(|pair| pair[0].0 >= pair[1].0)
                    {
                        return Err(CanonicalError::InvalidAssemblySolvePublication);
                    }
                    for (id, transform) in transforms {
                        validate_transform(*transform)?;
                        if product.grounded_occurrences.contains(id) {
                            return Err(CanonicalError::InvalidAssemblySolvePublication);
                        }
                        let existing = product
                            .occurrences
                            .get(id)
                            .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                        product.occurrences.insert(
                            *id,
                            Arc::new(Occurrence {
                                transform: *transform,
                                ..existing.as_ref().clone()
                            }),
                        );
                    }
                }
                CanonicalCommand::SetOccurrenceGrounded { id, grounded } => {
                    if !product.occurrences.contains_key(id) {
                        return Err(CanonicalError::OccurrenceNotFound(*id));
                    }
                    if *grounded {
                        product.grounded_occurrences.insert(*id);
                    } else {
                        product.grounded_occurrences.remove(id);
                    }
                }
                CanonicalCommand::CreateAssemblyMate(mate) => {
                    ensure_product_id(mate.id().0)?;
                    if product.assembly_mates.contains_key(&mate.id()) {
                        return Err(CanonicalError::AssemblyMateAlreadyExists(mate.id()));
                    }
                    validate_assembly_mate(&product, mate, true)?;
                    product
                        .assembly_mates
                        .insert(mate.id(), Arc::new(mate.clone()));
                }
                CanonicalCommand::RebindAssemblyMate(mate) => {
                    let existing = product
                        .assembly_mates
                        .get(&mate.id())
                        .ok_or(CanonicalError::AssemblyMateNotFound(mate.id()))?;
                    if existing.kind() != mate.kind() {
                        return Err(CanonicalError::InvalidAssemblyMate(mate.id()));
                    }
                    validate_assembly_mate(&product, mate, false)?;
                    product
                        .assembly_mates
                        .insert(mate.id(), Arc::new(mate.clone()));
                }
                CanonicalCommand::SetAssemblyMateKind { id, kind } => {
                    if !kind.is_valid() {
                        return Err(CanonicalError::InvalidAssemblyMate(*id));
                    }
                    let existing = product
                        .assembly_mates
                        .get(id)
                        .ok_or(CanonicalError::AssemblyMateNotFound(*id))?;
                    let replacement = AssemblyMate {
                        kind: *kind,
                        ..existing.as_ref().clone()
                    };
                    validate_assembly_mate(&product, &replacement, true)?;
                    product.assembly_mates.insert(*id, Arc::new(replacement));
                }
                CanonicalCommand::DeleteAssemblyMate { id } => {
                    product
                        .assembly_mates
                        .remove(id)
                        .ok_or(CanonicalError::AssemblyMateNotFound(*id))?;
                }
                CanonicalCommand::CreateAssemblyJoint(joint) => {
                    ensure_product_id(joint.id().0)?;
                    if product.assembly_joints.contains_key(&joint.id()) {
                        return Err(CanonicalError::AssemblyJointAlreadyExists(joint.id()));
                    }
                    validate_assembly_joint(&product, joint)?;
                    product
                        .assembly_joints
                        .insert(joint.id(), Arc::new(joint.clone()));
                }
                CanonicalCommand::SetAssemblyJointKind { id, kind } => {
                    let existing = product
                        .assembly_joints
                        .get(id)
                        .ok_or(CanonicalError::AssemblyJointNotFound(*id))?;
                    let replacement = AssemblyJoint {
                        kind: *kind,
                        ..existing.as_ref().clone()
                    };
                    validate_assembly_joint(&product, &replacement)?;
                    product.assembly_joints.insert(*id, Arc::new(replacement));
                }
                CanonicalCommand::SetAssemblyJointPosition { id, position } => {
                    let existing = product
                        .assembly_joints
                        .get(id)
                        .ok_or(CanonicalError::AssemblyJointNotFound(*id))?;
                    let kind = existing
                        .kind()
                        .with_position(*position)
                        .ok_or(CanonicalError::InvalidAssemblyJoint(*id))?;
                    let replacement = AssemblyJoint {
                        kind,
                        ..existing.as_ref().clone()
                    };
                    validate_assembly_joint(&product, &replacement)?;
                    product.assembly_joints.insert(*id, Arc::new(replacement));
                }
                CanonicalCommand::SetAssemblyJointLimits { id, limits } => {
                    let existing = product
                        .assembly_joints
                        .get(id)
                        .ok_or(CanonicalError::AssemblyJointNotFound(*id))?;
                    let kind = existing
                        .kind()
                        .with_limits(*limits)
                        .ok_or(CanonicalError::InvalidAssemblyJoint(*id))?;
                    let replacement = AssemblyJoint {
                        kind,
                        ..existing.as_ref().clone()
                    };
                    validate_assembly_joint(&product, &replacement)?;
                    product.assembly_joints.insert(*id, Arc::new(replacement));
                }
                CanonicalCommand::DeleteAssemblyJoint { id } => {
                    if product.assembly_motion_studies.values().any(|study| {
                        study
                            .drivers()
                            .iter()
                            .any(|driver| driver.joint_id() == *id)
                    }) {
                        return Err(CanonicalError::AssemblyJointInMotionStudy(*id));
                    }
                    if product.assembly_motion_couplings.values().any(|coupling| {
                        coupling.input_joint_id() == *id || coupling.output_joint_id() == *id
                    }) {
                        return Err(CanonicalError::AssemblyJointInMotionCoupling(*id));
                    }
                    product
                        .assembly_joints
                        .remove(id)
                        .ok_or(CanonicalError::AssemblyJointNotFound(*id))?;
                }
                CanonicalCommand::CreateAssemblyMotionCoupling(coupling) => {
                    ensure_product_id(coupling.id().0)?;
                    if product
                        .assembly_motion_couplings
                        .contains_key(&coupling.id())
                    {
                        return Err(CanonicalError::AssemblyMotionCouplingAlreadyExists(
                            coupling.id(),
                        ));
                    }
                    validate_assembly_motion_coupling(&product, coupling)?;
                    product
                        .assembly_motion_couplings
                        .insert(coupling.id(), Arc::new(coupling.clone()));
                }
                CanonicalCommand::UpdateAssemblyMotionCoupling(coupling) => {
                    if !product
                        .assembly_motion_couplings
                        .contains_key(&coupling.id())
                    {
                        return Err(CanonicalError::AssemblyMotionCouplingNotFound(
                            coupling.id(),
                        ));
                    }
                    validate_assembly_motion_coupling(&product, coupling)?;
                    product
                        .assembly_motion_couplings
                        .insert(coupling.id(), Arc::new(coupling.clone()));
                }
                CanonicalCommand::DeleteAssemblyMotionCoupling { id } => {
                    product
                        .assembly_motion_couplings
                        .remove(id)
                        .ok_or(CanonicalError::AssemblyMotionCouplingNotFound(*id))?;
                }
                CanonicalCommand::CreateAssemblyMotionStudy(study) => {
                    ensure_product_id(study.id().0)?;
                    if product.assembly_motion_studies.contains_key(&study.id()) {
                        return Err(CanonicalError::AssemblyMotionStudyAlreadyExists(study.id()));
                    }
                    validate_assembly_motion_study(&product, study)?;
                    product
                        .assembly_motion_studies
                        .insert(study.id(), Arc::new(study.clone()));
                }
                CanonicalCommand::UpdateAssemblyMotionStudy(study) => {
                    if !product.assembly_motion_studies.contains_key(&study.id()) {
                        return Err(CanonicalError::AssemblyMotionStudyNotFound(study.id()));
                    }
                    validate_assembly_motion_study(&product, study)?;
                    product
                        .assembly_motion_studies
                        .insert(study.id(), Arc::new(study.clone()));
                }
                CanonicalCommand::DeleteAssemblyMotionStudy { id } => {
                    product
                        .assembly_motion_studies
                        .remove(id)
                        .ok_or(CanonicalError::AssemblyMotionStudyNotFound(*id))?;
                }
                CanonicalCommand::CreateDrawingSheet(sheet) => {
                    if product.drawing_sheets.contains_key(&sheet.id()) {
                        return Err(CanonicalError::DrawingSheetAlreadyExists(sheet.id()));
                    }
                    validate_drawing_sheet(&product, sheet)?;
                    product
                        .drawing_sheets
                        .insert(sheet.id(), Arc::new(sheet.clone()));
                }
                CanonicalCommand::UpdateDrawingSheet(sheet) => {
                    if !product.drawing_sheets.contains_key(&sheet.id()) {
                        return Err(CanonicalError::DrawingSheetNotFound(sheet.id()));
                    }
                    validate_drawing_sheet(&product, sheet)?;
                    product
                        .drawing_sheets
                        .insert(sheet.id(), Arc::new(sheet.clone()));
                }
                CanonicalCommand::DeleteDrawingSheet { id } => {
                    product
                        .drawing_sheets
                        .remove(id)
                        .ok_or(CanonicalError::DrawingSheetNotFound(*id))?;
                }
                CanonicalCommand::SetOccurrenceVisibility { id, visible } => {
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            visible: *visible,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::SetOccurrenceTag { id, tag } => {
                    if let Some(tag_id) = tag
                        && !product.tags.contains_key(tag_id)
                    {
                        return Err(CanonicalError::TagNotFound(*tag_id));
                    }
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            tag: *tag,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::RepointOccurrence { id, definition_id } => {
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            definition_id: *definition_id,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::SetOccurrenceParent { id, parent } => {
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            parent: *parent,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::CreateGroup {
                    id,
                    name,
                    transform,
                    parent,
                } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    validate_transform(*transform)?;
                    if product.groups.contains_key(id) {
                        return Err(CanonicalError::GroupAlreadyExists(*id));
                    }
                    product.groups.insert(
                        *id,
                        Arc::new(Group {
                            id: *id,
                            name: name.clone(),
                            transform: *transform,
                            parent: *parent,
                        }),
                    );
                }
                CanonicalCommand::DeleteGroup { id } => {
                    if product
                        .occurrences
                        .values()
                        .any(|occurrence| occurrence.parent == Some(*id))
                        || product
                            .groups
                            .values()
                            .any(|group| group.parent == Some(*id))
                    {
                        return Err(CanonicalError::GroupNotEmpty(*id));
                    }
                    product
                        .groups
                        .remove(id)
                        .ok_or(CanonicalError::GroupNotFound(*id))?;
                }
                CanonicalCommand::SetGroupTransform { id, transform } => {
                    validate_transform(*transform)?;
                    let existing = product
                        .groups
                        .get(id)
                        .ok_or(CanonicalError::GroupNotFound(*id))?;
                    product.groups.insert(
                        *id,
                        Arc::new(Group {
                            transform: *transform,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::SetGroupParent { id, parent } => {
                    let existing = product
                        .groups
                        .get(id)
                        .ok_or(CanonicalError::GroupNotFound(*id))?;
                    product.groups.insert(
                        *id,
                        Arc::new(Group {
                            parent: *parent,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                    clone_definition_and_repoint(&mut product, plan)?;
                }
                CanonicalCommand::ConvertGroupToComponent(plan) => {
                    convert_group_to_component_model(&mut product, plan)?;
                }
                CanonicalCommand::ApplySolidTool(plan) => {
                    apply_solid_tool(&mut product, plan)?;
                }
            }
        }

        refresh_supported_planar_face_frames(&mut product, Some(&current))?;
        let mut anchored_reference_lineages = BTreeSet::new();
        let mut stale_reference_lineages = BTreeSet::new();
        for feature in product.features.values() {
            let references = match &feature.kind {
                FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace { reference, .. },
                    ..
                }) => vec![reference.as_ref()],
                FeatureKind::Pad(spec) => spec.extent.references(),
                FeatureKind::SketchPocket(spec) => std::iter::once(spec.support.as_ref())
                    .chain(spec.extent.references())
                    .collect(),
                _ => Vec::new(),
            };
            for reference in references {
                let mut producer_dependencies = BTreeSet::new();
                add_feature_dependency_closure(
                    &current,
                    reference.producer_feature_id,
                    &mut producer_dependencies,
                );
                let producer_inputs_unchanged = producer_dependencies.iter().all(|dependency| {
                    let AuthoritativeDependency::Feature(id) = dependency else {
                        return true;
                    };
                    current
                        .feature(*id)
                        .zip(product.features.get(id))
                        .is_some_and(|(before, after)| before.kind() == &after.kind)
                });
                if !producer_inputs_unchanged {
                    stale_reference_lineages.insert(reference.lineage_digest.clone());
                }
                anchored_reference_lineages.insert(reference.lineage_digest.clone());
            }
        }
        for lineage in &stale_reference_lineages {
            set_planar_face_reference_health(&mut product, lineage, WorkplaneSupportHealth::Stale);
            product.exact_reference_evidence.remove(lineage);
        }
        product
            .exact_reference_evidence
            .retain(|lineage, _| anchored_reference_lineages.contains(lineage));

        validate_graph(&product.evaluator_nodes)?;
        refresh_override_health(&mut product);
        validate_overrides(&product)?;
        validate_product(&product)?;
        validate_assembly_joint_motion_publication(&current, &product, batch)?;
        let revision_id = self.next_revision_id;
        let following_revision_id = revision_id
            .checked_add(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let recomputed_nodes =
            dependent_closure(&product.evaluator_nodes, &changed_evaluator_nodes);
        let evaluation = evaluate_affected(
            &product.evaluator_nodes,
            &evaluation_identity,
            previous_evaluation.as_ref(),
            &recomputed_nodes,
        )
        .map_err(CanonicalError::Graph)?;
        let changed_features = current
            .product
            .features
            .keys()
            .chain(product.features.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|id| current.product.features.get(id) != product.features.get(id))
            .collect::<BTreeSet<_>>();
        let feature_graph = FeatureDependencyGraph::from_product(&product)?;
        let dirty_features = feature_graph.dependent_closure(
            changed_features
                .iter()
                .chain(&explicit_dirty_features)
                .cloned()
                .filter(|id| product.features.contains_key(id)),
        );
        let feature_states = feature_graph.evaluation_states(&dirty_features, &BTreeSet::new());
        let snapshot = Snapshot {
            revision_id,
            product: Arc::new(product),
        };
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot,
            batch_digest: batch.digest(),
            recomputed_nodes,
            dirty_features,
            feature_states,
            evaluation: Some(evaluation),
        });

        self.revisions.truncate(self.cursor + 1);
        self.revisions.push(Arc::clone(&revision));
        self.cursor += 1;
        self.next_revision_id = following_revision_id;
        Ok(revision)
    }

    pub fn make_unique(
        &mut self,
        occurrence_id: OccurrenceId,
        new_definition_name: impl Into<String>,
    ) -> Result<Arc<Revision>, CanonicalError> {
        let snapshot = self.current();
        let occurrence = snapshot
            .occurrence(occurrence_id)
            .ok_or(CanonicalError::OccurrenceNotFound(occurrence_id))?;
        let source = snapshot
            .definition(occurrence.definition_id)
            .ok_or(CanonicalError::DefinitionNotFound(occurrence.definition_id))?;
        let new_definition_id =
            DefinitionId(next_id(snapshot.definitions().map(|item| item.id.0))?);
        let mut next_feature_id = next_id(snapshot.features().map(|item| item.id.0))?;
        let feature_id_map = source
            .feature_ids
            .iter()
            .map(|source_id| {
                let mapped = FeatureId(next_feature_id);
                next_feature_id += 1;
                (*source_id, mapped)
            })
            .collect();
        let plan = CloneDefinitionPlan {
            occurrence_id,
            source_definition_id: occurrence.definition_id,
            new_definition_id,
            new_definition_name: new_definition_name.into(),
            feature_id_map,
        };
        self.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CloneDefinitionAndRepoint(plan),
        ]))
    }

    pub fn convert_group_to_component(
        &mut self,
        group_id: GroupId,
        component_name: impl Into<String>,
    ) -> Result<ConvertGroupToComponentResult, CanonicalError> {
        let snapshot = self.current();
        if snapshot.group(group_id).is_none() {
            return Err(CanonicalError::GroupNotFound(group_id));
        }
        let plan = ConvertGroupPlan {
            group_id,
            new_definition_id: DefinitionId(next_id(snapshot.definitions().map(|item| item.id.0))?),
            new_occurrence_id: OccurrenceId(next_id(snapshot.occurrences().map(|item| item.id.0))?),
            component_name: component_name.into(),
        };
        let mappings = conversion_mappings(snapshot.product(), &plan)?;
        let component_definition_id = plan.new_definition_id;
        let component_occurrence_id = plan.new_occurrence_id;
        let revision = self.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::ConvertGroupToComponent(plan),
        ]))?;
        Ok(ConvertGroupToComponentResult {
            revision,
            component_definition_id,
            component_occurrence_id,
            mappings,
        })
    }

    pub fn register_evaluation(
        &mut self,
        root_rule_node_id: NodeId,
        slot_path: SlotPath,
        report: &EvaluationReport,
    ) -> Result<DerivedResultEvent, CanonicalError> {
        let snapshot = self.current();
        let Some(root_node) = snapshot.evaluator_node(root_rule_node_id) else {
            return Err(CanonicalError::NodeNotFound(root_rule_node_id));
        };
        if !matches!(root_node.kind(), EvaluatorNodeKind::Rule { .. }) {
            return Err(CanonicalError::WrongNodeKind(root_rule_node_id));
        }
        let target = DerivedIdentity::new(root_rule_node_id, slot_path.clone())
            .map_err(CanonicalError::Graph)?;
        if resolve_derived_identity(&snapshot.product.evaluator_nodes, &target)
            != SlotResolution::Resolved
        {
            return Err(CanonicalError::UnresolvedDerivedOutput);
        }
        if report.document_id != Some(snapshot.document_id())
            || report.revision_id != Some(snapshot.revision_id())
            || report.canonical_digest.as_deref() != Some(snapshot.canonical_digest().as_str())
        {
            return Err(CanonicalError::EvaluationEnvelopeMismatch);
        }
        let expected = snapshot.evaluate(&report.identity)?;
        let supplied_node = report
            .node(root_rule_node_id)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        let expected_node = expected
            .node(root_rule_node_id)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        if supplied_node != expected_node {
            return Err(CanonicalError::EvaluationEvidenceMismatch);
        }
        if !matches!(supplied_node.status, EvaluationStatus::Evaluated(_)) {
            return Err(CanonicalError::FailedEvaluation(root_rule_node_id));
        }
        let output = report
            .outputs
            .get(&target)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        let expected_output = expected
            .outputs
            .get(&target)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        if output != expected_output {
            return Err(CanonicalError::EvaluationEvidenceMismatch);
        }
        let key = DerivedResultKey {
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            root_rule_node_id,
            slot_path,
            input_digest: output.input_digest.clone(),
            result_digest: output.result_digest.clone(),
            evaluator: report.identity.evaluator.clone(),
            backend: report.identity.backend.clone(),
            schema: report.identity.schema.clone(),
            tolerance: report.identity.tolerance.clone(),
        };
        let event = DerivedResultEvent {
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            classification: DerivedResultClassification::Current,
            payload: DerivedResultPayload::Evaluation(key),
        };
        if !self.register_derived_result(event.clone()) {
            return Err(CanonicalError::EvaluationEnvelopeMismatch);
        }
        Ok(event)
    }

    #[must_use]
    pub fn evaluation_registry_len(&self) -> usize {
        self.evaluation_registry.len()
    }

    pub fn undo(&mut self) -> Option<Snapshot> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        Some(self.current())
    }

    pub fn redo(&mut self) -> Option<Snapshot> {
        if self.cursor + 1 >= self.revisions.len() {
            return None;
        }
        self.cursor += 1;
        Some(self.current())
    }

    /// Captures the immediate parent of the current last tip for correction planning.
    pub fn tip_replacement_parent(
        &self,
    ) -> Result<TipReplacementParent, TipReplacementProposalError> {
        if self.cursor == 0 {
            return Err(TipReplacementProposalError::NoCurrentTip);
        }
        if self.cursor + 1 != self.revisions.len() {
            return Err(TipReplacementProposalError::RedoBranch);
        }
        let superseded = &self.revisions[self.cursor].snapshot;
        let corrected_revision =
            superseded
                .revision_id
                .checked_add(1)
                .ok_or(TipReplacementProposalError::Canonical(
                    CanonicalError::RevisionExhausted,
                ))?;
        if self.next_revision_id != corrected_revision {
            return Err(TipReplacementProposalError::Stale);
        }
        let parent = self.revisions[self.cursor - 1].snapshot.clone();
        Ok(TipReplacementParent {
            snapshot: parent.clone(),
            document_id: superseded.document_id(),
            parent_revision: parent.revision_id(),
            parent_digest: parent.canonical_digest(),
            superseded_revision: superseded.revision_id(),
            superseded_digest: superseded.canonical_digest(),
        })
    }

    /// Prepares a correction against a previously guarded immediate-parent snapshot.
    pub fn prepare_tip_replacement_proposal(
        &self,
        parent: &TipReplacementParent,
        batch: CommandBatch,
        context: ProposalContext,
    ) -> Result<TipReplacementProposal, TipReplacementProposalError> {
        validate_confirmation_requirement(&context)
            .map_err(TipReplacementProposalError::Preparation)?;
        let current_parent = self.validate_tip_replacement_envelope(
            parent.document_id,
            parent.parent_revision,
            &parent.parent_digest,
            parent.superseded_revision,
            &parent.superseded_digest,
            parent.superseded_revision.checked_add(1).ok_or(
                TipReplacementProposalError::Canonical(CanonicalError::RevisionExhausted),
            )?,
        )?;
        if parent.snapshot.document_id() != parent.document_id
            || parent.snapshot.revision_id() != parent.parent_revision
            || parent.snapshot.canonical_digest() != parent.parent_digest
            || parent.snapshot.canonical_digest() != current_parent.canonical_digest()
        {
            return Err(TipReplacementProposalError::Stale);
        }

        let authoritative_dependencies = authoritative_dependencies(&current_parent, &batch);
        let authoritative_writes = authoritative_writes(&current_parent, &batch);
        let cost = ProposalCost {
            commands: batch.commands.len(),
            read_dependencies: authoritative_dependencies.len(),
            write_targets: authoritative_writes.len(),
        };
        validate_proposal_budget(context.requested_budget, cost)
            .map_err(TipReplacementProposalError::Preparation)?;
        let corrected_revision = parent.superseded_revision + 1;
        let (_, authoritative_diff, intended_result_digest) = tip_replacement_candidate(
            &current_parent,
            corrected_revision,
            &batch,
            &authoritative_writes,
            context.goal.clone(),
        )
        .map_err(TipReplacementProposalError::Preparation)?;
        Ok(TipReplacementProposal {
            document_id: parent.document_id,
            parent_revision: parent.parent_revision,
            parent_digest: parent.parent_digest.clone(),
            superseded_revision: parent.superseded_revision,
            superseded_digest: parent.superseded_digest.clone(),
            corrected_revision,
            command_digest: batch.digest(),
            dependency_digest: dependency_digest(&current_parent, &authoritative_dependencies),
            authoritative_dependencies,
            authoritative_writes,
            authoritative_diff,
            intended_result_digest,
            principal: context.principal,
            goal: context.goal,
            assumptions: context.assumptions,
            risk: context.risk,
            confirmation: context.confirmation,
            requested_budget: context.requested_budget,
            cost,
            batch,
        })
    }

    /// Replays and verifies a correction without changing document state.
    pub fn preview_tip_replacement_proposal(
        &self,
        proposal: &TipReplacementProposal,
    ) -> Result<Snapshot, TipReplacementProposalError> {
        self.verify_tip_replacement_proposal(proposal)
    }

    /// Atomically replaces the exact current last tip with its verified correction.
    pub fn commit_tip_replacement_proposal(
        &mut self,
        proposal: &TipReplacementProposal,
    ) -> Result<Arc<Revision>, TipReplacementProposalError> {
        if matches!(proposal.risk, ProposalRisk::High(_)) {
            return Err(TipReplacementProposalError::HumanApprovalRequired);
        }
        let expected = self.verify_tip_replacement_proposal(proposal)?;
        let parent = self.revisions[self.cursor - 1].snapshot.clone();
        let previous_revisions = self.revisions.clone();
        let previous_cursor = self.cursor;
        let previous_next_revision_id = self.next_revision_id;
        let previous_registry = self.evaluation_registry.clone();

        self.revisions.pop();
        self.cursor -= 1;
        let result = self
            .apply_batch(&proposal.batch)
            .map_err(TipReplacementProposalError::Canonical)
            .and_then(|revision| {
                let actual_diff: Vec<_> = proposal
                    .authoritative_writes
                    .iter()
                    .cloned()
                    .map(|target| ProposalDiffEntry {
                        target: target.clone(),
                        before: proposal_value(&parent, target.clone(), proposal.goal.clone()),
                        after: proposal_value(revision.snapshot(), target, proposal.goal.clone()),
                    })
                    .collect();
                let actual_result_digest =
                    dependency_digest(revision.snapshot(), &proposal.authoritative_writes);
                if revision.id() != proposal.corrected_revision
                    || revision.batch_digest() != proposal.command_digest
                    || actual_diff != proposal.authoritative_diff
                    || actual_result_digest != proposal.intended_result_digest
                    || revision.snapshot().canonical_digest() != expected.canonical_digest()
                {
                    return Err(TipReplacementProposalError::VerificationMismatch);
                }
                Ok(revision)
            });

        match result {
            Ok(revision) => {
                self.evaluation_registry
                    .retain(|key, _| key.revision_id != proposal.superseded_revision);
                Ok(revision)
            }
            Err(error) => {
                self.revisions = previous_revisions;
                self.cursor = previous_cursor;
                self.next_revision_id = previous_next_revision_id;
                self.evaluation_registry = previous_registry;
                Err(error)
            }
        }
    }

    fn validate_tip_replacement_envelope(
        &self,
        document_id: DocumentId,
        parent_revision: u64,
        parent_digest: &str,
        superseded_revision: u64,
        superseded_digest: &str,
        corrected_revision: u64,
    ) -> Result<Snapshot, TipReplacementProposalError> {
        if self.cursor == 0 {
            return Err(TipReplacementProposalError::NoCurrentTip);
        }
        if self.cursor + 1 != self.revisions.len() {
            return Err(TipReplacementProposalError::RedoBranch);
        }
        let current = &self.revisions[self.cursor].snapshot;
        let parent = &self.revisions[self.cursor - 1].snapshot;
        if current.document_id() != document_id
            || current.revision_id() != superseded_revision
            || current.canonical_digest() != superseded_digest
            || parent.document_id() != document_id
            || parent.revision_id() != parent_revision
            || parent.canonical_digest() != parent_digest
            || superseded_revision.checked_add(1) != Some(corrected_revision)
            || self.next_revision_id != corrected_revision
        {
            return Err(TipReplacementProposalError::Stale);
        }
        Ok(parent.clone())
    }

    fn verify_tip_replacement_proposal(
        &self,
        proposal: &TipReplacementProposal,
    ) -> Result<Snapshot, TipReplacementProposalError> {
        let parent = self.validate_tip_replacement_envelope(
            proposal.document_id,
            proposal.parent_revision,
            &proposal.parent_digest,
            proposal.superseded_revision,
            &proposal.superseded_digest,
            proposal.corrected_revision,
        )?;
        if proposal.command_digest != proposal.batch.digest() {
            return Err(TipReplacementProposalError::VerificationMismatch);
        }
        let authoritative_dependencies = authoritative_dependencies(&parent, &proposal.batch);
        let authoritative_writes = authoritative_writes(&parent, &proposal.batch);
        let cost = ProposalCost {
            commands: proposal.batch.commands.len(),
            read_dependencies: authoritative_dependencies.len(),
            write_targets: authoritative_writes.len(),
        };
        if authoritative_dependencies != proposal.authoritative_dependencies
            || authoritative_writes != proposal.authoritative_writes
            || dependency_digest(&parent, &authoritative_dependencies) != proposal.dependency_digest
            || cost != proposal.cost
        {
            return Err(TipReplacementProposalError::VerificationMismatch);
        }
        validate_proposal_budget(proposal.requested_budget, cost)
            .map_err(TipReplacementProposalError::Preparation)?;
        validate_confirmation_requirement(&ProposalContext {
            principal: proposal.principal,
            goal: proposal.goal.clone(),
            assumptions: proposal.assumptions.clone(),
            risk: proposal.risk,
            confirmation: proposal.confirmation.clone(),
            requested_budget: proposal.requested_budget,
        })
        .map_err(TipReplacementProposalError::Preparation)?;
        let (preview, authoritative_diff, intended_result_digest) = tip_replacement_candidate(
            &parent,
            proposal.corrected_revision,
            &proposal.batch,
            &authoritative_writes,
            proposal.goal.clone(),
        )
        .map_err(TipReplacementProposalError::Preparation)?;
        if authoritative_diff != proposal.authoritative_diff
            || intended_result_digest != proposal.intended_result_digest
        {
            return Err(TipReplacementProposalError::VerificationMismatch);
        }
        Ok(preview)
    }

    pub fn plan_pad_pocket(
        &self,
        id: FeatureId,
        definition_id: DefinitionId,
        name: impl Into<String>,
        operation: PadPocketOperation,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        let kind = match operation {
            PadPocketOperation::Pad(spec) => FeatureKind::Pad(spec),
            PadPocketOperation::Pocket(spec) => FeatureKind::SketchPocket(spec),
        };
        self.prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id,
                definition_id,
                name: name.into(),
                kind,
            }]),
            context,
        )
    }

    pub fn plan_new_body_feature(
        &self,
        plan: NewBodyFeaturePlan,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        if !matches!(
            plan.feature_kind,
            FeatureKind::Extrusion { .. } | FeatureKind::Pad(_)
        ) {
            return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
        }
        let snapshot = self.current();
        for dependency in plan.feature_kind.authoritative_dependencies() {
            let feature = snapshot
                .feature(dependency)
                .ok_or(CanonicalError::FeatureNotFound(dependency))?;
            if feature.definition_id() != plan.definition_id
                || !feature_references_are_resolved(
                    snapshot.product(),
                    dependency,
                    &mut BTreeSet::new(),
                )
            {
                return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
            }
        }
        self.prepare_proposal_with_context(
            CommandBatch::new(vec![
                CanonicalCommand::CreateBody {
                    definition_id: plan.definition_id,
                    id: plan.body_id,
                    name: plan.body_name,
                    visible: true,
                },
                CanonicalCommand::SetActiveBody {
                    definition_id: plan.definition_id,
                    id: plan.body_id,
                },
                CanonicalCommand::CreateFeature {
                    id: plan.feature_id,
                    definition_id: plan.definition_id,
                    name: plan.feature_name,
                    kind: plan.feature_kind,
                },
            ]),
            context,
        )
    }

    pub fn plan_multibody_boolean(
        &self,
        plan: MultiBodyBooleanPlan,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        if plan.target_body_id == plan.tool_body_id
            || plan.target_feature_id == plan.tool_feature_id
            || plan.operation == BooleanOperation::Split
        {
            return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
        }
        let snapshot = self.current();
        let definition = snapshot
            .definition(plan.definition_id)
            .ok_or(CanonicalError::DefinitionNotFound(plan.definition_id))?;
        for body_id in [plan.target_body_id, plan.tool_body_id] {
            let body = definition
                .body(body_id)
                .ok_or(CanonicalError::BodyNotFound(plan.definition_id, body_id))?;
            if body.consumed_by().is_some() {
                return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
            }
        }
        for (feature_id, body_id) in [
            (plan.target_feature_id, plan.target_body_id),
            (plan.tool_feature_id, plan.tool_body_id),
        ] {
            let feature = snapshot
                .feature(feature_id)
                .ok_or(CanonicalError::FeatureNotFound(feature_id))?;
            if feature.definition_id() != plan.definition_id
                || !matches!(
                    feature.kind(),
                    FeatureKind::Extrusion { .. } | FeatureKind::Pad(_)
                )
                || definition
                    .feature_body_ownership(feature_id)
                    .and_then(FeatureBodyOwnership::output_body_id)
                    != Some(body_id)
                || !feature_references_are_resolved(
                    snapshot.product(),
                    feature_id,
                    &mut BTreeSet::new(),
                )
                || definition
                    .feature_ids()
                    .iter()
                    .cloned()
                    .any(|dependent_id| {
                        dependent_id != feature_id
                            && snapshot.feature(dependent_id).is_some_and(|dependent| {
                                dependent.kind().dependencies().contains(&feature_id)
                                    && definition
                                        .feature_body_ownership(dependent_id)
                                        .and_then(FeatureBodyOwnership::output_body_id)
                                        == Some(body_id)
                            })
                    })
            {
                return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
            }
        }
        let mut commands = vec![
            CanonicalCommand::SetActiveBody {
                definition_id: plan.definition_id,
                id: plan.target_body_id,
            },
            CanonicalCommand::CreateFeature {
                id: plan.result_feature_id,
                definition_id: plan.definition_id,
                name: plan.result_feature_name,
                kind: FeatureKind::Boolean {
                    operation: plan.operation,
                    target: plan.target_feature_id,
                    tool: plan.tool_feature_id,
                },
            },
        ];
        if plan.tool_policy == ToolBodyPolicy::Consume {
            commands.push(CanonicalCommand::ConsumeBody {
                definition_id: plan.definition_id,
                id: plan.tool_body_id,
                by_feature_id: plan.result_feature_id,
            });
        }
        self.prepare_proposal_with_context(CommandBatch::new(commands), context)
    }

    pub fn plan_body_command(
        &self,
        command: CanonicalCommand,
    ) -> Result<Proposal, ProposalPrepareError> {
        if !matches!(
            command,
            CanonicalCommand::CreateBody { .. }
                | CanonicalCommand::DeleteBody { .. }
                | CanonicalCommand::RenameBody { .. }
                | CanonicalCommand::SetActiveBody { .. }
                | CanonicalCommand::SetBodyVisibility { .. }
                | CanonicalCommand::ConsumeBody { .. }
                | CanonicalCommand::SetFeatureBodyOwnership { .. }
        ) {
            return Err(ProposalPrepareError::Canonical(
                CanonicalError::InvalidBodyCommand,
            ));
        }
        self.prepare_proposal(CommandBatch::new(vec![command]))
    }

    pub fn prepare_proposal(&self, batch: CommandBatch) -> Result<Proposal, ProposalPrepareError> {
        self.prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
    }

    pub fn prepare_proposal_with_context(
        &self,
        batch: CommandBatch,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        validate_confirmation_requirement(&context)?;
        let snapshot = self.current();
        let authoritative_dependencies = authoritative_dependencies(&snapshot, &batch);
        let authoritative_writes = authoritative_writes(&snapshot, &batch);
        let cost = ProposalCost {
            commands: batch.commands.len(),
            read_dependencies: authoritative_dependencies.len(),
            write_targets: authoritative_writes.len(),
        };
        validate_proposal_budget(context.requested_budget, cost)?;
        let (authoritative_diff, intended_result_digest) = proposal_candidate(
            &snapshot,
            &batch,
            &authoritative_writes,
            context.goal.clone(),
        )?;
        Ok(Proposal {
            document_id: snapshot.document_id(),
            provenance_revision: snapshot.revision_id,
            provenance_digest: snapshot.canonical_digest(),
            command_digest: batch.digest(),
            dependency_digest: dependency_digest(&snapshot, &authoritative_dependencies),
            authoritative_dependencies,
            authoritative_writes,
            authoritative_diff,
            intended_result_digest,
            principal: context.principal,
            goal: context.goal,
            assumptions: context.assumptions,
            risk: context.risk,
            confirmation: context.confirmation,
            requested_budget: context.requested_budget,
            cost,
            batch,
        })
    }

    pub fn prepare_high_risk_side_effect(
        &self,
        operation: &str,
        principal: ProposalPrincipal,
        scope: HighRiskScope,
        payload: &[u8],
    ) -> Result<SideEffectProposal, HumanConfirmationError> {
        if operation.is_empty()
            || operation.len() > 128
            || operation.chars().any(char::is_control)
            || payload.is_empty()
        {
            return Err(HumanConfirmationError::InvalidSideEffectEvidence);
        }
        validate_high_risk_requester(principal)?;
        let snapshot = self.current();
        let payload_digest = format!("{:x}", Sha256::digest(payload));
        let mut evidence = Vec::new();
        push_confirmation_field(&mut evidence, b"ketchup.side-effect-proposal.v1");
        push_confirmation_field(&mut evidence, operation.as_bytes());
        push_confirmation_u64(&mut evidence, snapshot.document_id().0);
        push_confirmation_u64(&mut evidence, snapshot.revision_id);
        push_confirmation_field(&mut evidence, snapshot.canonical_digest().as_bytes());
        push_confirmation_principal(&mut evidence, principal);
        push_confirmation_scope(&mut evidence, &scope);
        push_confirmation_field(&mut evidence, payload_digest.as_bytes());
        let operation_digest = format!("{:x}", Sha256::digest(&evidence));
        Ok(SideEffectProposal {
            document_id: snapshot.document_id(),
            provenance_revision: snapshot.revision_id,
            provenance_digest: snapshot.canonical_digest(),
            operation: operation.to_owned(),
            operation_digest,
            payload_digest,
            principal,
            scope,
        })
    }

    #[must_use]
    pub fn validate_proposal(&self, proposal: &Proposal) -> ProposalValidity {
        let snapshot = self.current();
        let envelope_matches = proposal.document_id == snapshot.document_id();
        let command_matches = proposal.command_digest == proposal.batch.digest();
        let dependencies_match = proposal.dependency_digest
            == dependency_digest(&snapshot, &proposal.authoritative_dependencies);
        if envelope_matches && command_matches && dependencies_match {
            ProposalValidity::Valid {
                evaluated_revision: snapshot.revision_id,
            }
        } else {
            ProposalValidity::Stale {
                provenance_revision: proposal.provenance_revision,
                current_revision: snapshot.revision_id,
            }
        }
    }

    pub fn commit_proposal(
        &mut self,
        proposal: &Proposal,
    ) -> Result<Arc<Revision>, ProposalCommitError> {
        self.commit_verified_proposal(proposal)
            .map(|committed| committed.revision)
    }

    pub fn commit_verified_proposal(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, ProposalCommitError> {
        if matches!(proposal.risk, ProposalRisk::High(_)) {
            return Err(ProposalCommitError::HumanApprovalRequired);
        }
        self.commit_verified_proposal_inner(proposal)
    }

    pub fn commit_high_risk_proposal(
        &mut self,
        proposal: &Proposal,
        approval: &HumanApprovalToken,
        now_ms: u64,
    ) -> Result<VerifiedProposalCommit, ProposalCommitError> {
        let ProposalRisk::High(risk_class) = proposal.risk else {
            return Err(ProposalCommitError::HumanApprovalUnexpected);
        };
        let ProposalConfirmation::HumanOnly(scope) = &proposal.confirmation else {
            return Err(ProposalCommitError::HumanApprovalInvalid);
        };
        let signature = approval.signature;
        {
            let policy = self
                .human_confirmation_policy
                .as_ref()
                .ok_or(ProposalCommitError::HumanApprovalPolicyUnavailable)?;
            if policy.epoch != approval.policy_epoch {
                return Err(ProposalCommitError::HumanApprovalPolicyStale);
            }
            if policy.consumed_signatures.contains(&signature) {
                return Err(ProposalCommitError::HumanApprovalReplayed);
            }
            if approval.approving_human == 0
                || approval.requester != proposal.principal
                || approval.document_id != proposal.document_id
                || approval.revision_id != proposal.provenance_revision
                || approval.provenance_digest != proposal.provenance_digest
                || approval.dependency_digest != proposal.dependency_digest
                || approval.command_digest != proposal.command_digest
                || approval.result_digest != proposal.intended_result_digest
                || approval.scope != *scope
                || approval.scope.class != risk_class
                || now_ms < approval.issued_at_ms
                || now_ms > approval.expires_at_ms
            {
                return Err(ProposalCommitError::HumanApprovalInvalid);
            }
            policy
                .verifying_key
                .verify(
                    &approval.signing_payload(),
                    &Signature::from_bytes(&approval.signature),
                )
                .map_err(|_| ProposalCommitError::HumanApprovalInvalid)?;
        }
        let committed = self.commit_verified_proposal_inner(proposal)?;
        self.human_confirmation_policy
            .as_mut()
            .expect("confirmation policy was verified before commit")
            .consumed_signatures
            .insert(signature);
        Ok(committed)
    }

    pub fn authorize_high_risk_side_effect(
        &mut self,
        proposal: &SideEffectProposal,
        approval: &SideEffectApprovalToken,
        now_ms: u64,
    ) -> Result<SideEffectAuthorizationReceipt, SideEffectAuthorizationError> {
        let signature = approval.signature;
        let snapshot = self.current();
        {
            let policy = self
                .human_confirmation_policy
                .as_ref()
                .ok_or(SideEffectAuthorizationError::PolicyUnavailable)?;
            if policy.epoch != approval.policy_epoch {
                return Err(SideEffectAuthorizationError::PolicyStale);
            }
            if policy.consumed_signatures.contains(&signature) {
                return Err(SideEffectAuthorizationError::Replayed);
            }
            if approval.approving_human == 0
                || approval.requester != proposal.principal
                || approval.document_id != proposal.document_id
                || approval.revision_id != proposal.provenance_revision
                || approval.provenance_digest != proposal.provenance_digest
                || approval.operation_digest != proposal.operation_digest
                || approval.payload_digest != proposal.payload_digest
                || approval.scope != proposal.scope
                || snapshot.document_id() != proposal.document_id
                || snapshot.revision_id != proposal.provenance_revision
                || snapshot.canonical_digest() != proposal.provenance_digest
                || now_ms < approval.issued_at_ms
                || now_ms > approval.expires_at_ms
            {
                return Err(SideEffectAuthorizationError::Invalid);
            }
            policy
                .verifying_key
                .verify(
                    &approval.signing_payload(),
                    &Signature::from_bytes(&approval.signature),
                )
                .map_err(|_| SideEffectAuthorizationError::Invalid)?;
        }
        self.human_confirmation_policy
            .as_mut()
            .expect("side-effect policy was verified before authorization")
            .consumed_signatures
            .insert(signature);
        Ok(SideEffectAuthorizationReceipt {
            approving_human: approval.approving_human,
            document_id: proposal.document_id,
            revision_id: proposal.provenance_revision,
            operation: proposal.operation.clone(),
            operation_digest: proposal.operation_digest.clone(),
            payload_digest: proposal.payload_digest.clone(),
            scope: proposal.scope.clone(),
            policy_epoch: approval.policy_epoch,
            authorized_at_ms: now_ms,
        })
    }

    fn commit_verified_proposal_inner(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, ProposalCommitError> {
        if let stale @ ProposalValidity::Stale { .. } = self.validate_proposal(proposal) {
            return Err(ProposalCommitError::Stale(stale));
        }
        let current = self.current();
        let (diff, intended_result_digest) = proposal_candidate(
            &current,
            &proposal.batch,
            &proposal.authoritative_writes,
            proposal.goal.clone(),
        )
        .map_err(ProposalCommitError::Preparation)?;
        if diff != proposal.authoritative_diff
            || intended_result_digest != proposal.intended_result_digest
        {
            return Err(ProposalCommitError::VerificationMismatch);
        }
        let previous_revisions = self.revisions.clone();
        let previous_cursor = self.cursor;
        let previous_next_revision_id = self.next_revision_id;
        let previous_registry = self.evaluation_registry.clone();
        let revision = self
            .apply_batch(&proposal.batch)
            .map_err(ProposalCommitError::Canonical)?;
        let actual_result_digest =
            dependency_digest(revision.snapshot(), &proposal.authoritative_writes);
        if revision.batch_digest() != proposal.command_digest
            || actual_result_digest != proposal.intended_result_digest
        {
            self.revisions = previous_revisions;
            self.cursor = previous_cursor;
            self.next_revision_id = previous_next_revision_id;
            self.evaluation_registry = previous_registry;
            return Err(ProposalCommitError::VerificationMismatch);
        }
        Ok(VerifiedProposalCommit {
            revision,
            command_digest: proposal.command_digest.clone(),
            result_digest: actual_result_digest,
            verified_writes: proposal.authoritative_writes.clone(),
        })
    }
}

/// An immutable immediate-parent snapshot guarded by the exact tip it may replace.
#[derive(Clone)]
pub struct TipReplacementParent {
    snapshot: Snapshot,
    document_id: DocumentId,
    parent_revision: u64,
    parent_digest: String,
    superseded_revision: u64,
    superseded_digest: String,
}

impl TipReplacementParent {
    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn parent_revision(&self) -> u64 {
        self.parent_revision
    }

    #[must_use]
    pub fn parent_digest(&self) -> &str {
        &self.parent_digest
    }

    #[must_use]
    pub const fn superseded_revision(&self) -> u64 {
        self.superseded_revision
    }

    #[must_use]
    pub fn superseded_digest(&self) -> &str {
        &self.superseded_digest
    }
}

/// A canonical correction planned against the parent of the current tip.
#[derive(Clone)]
pub struct TipReplacementProposal {
    document_id: DocumentId,
    parent_revision: u64,
    parent_digest: String,
    superseded_revision: u64,
    superseded_digest: String,
    corrected_revision: u64,
    batch: CommandBatch,
    command_digest: String,
    authoritative_dependencies: BTreeSet<AuthoritativeDependency>,
    authoritative_writes: BTreeSet<AuthoritativeDependency>,
    dependency_digest: String,
    authoritative_diff: Vec<ProposalDiffEntry>,
    intended_result_digest: String,
    principal: ProposalPrincipal,
    goal: ProposalGoal,
    assumptions: Vec<ProposalAssumption>,
    risk: ProposalRisk,
    confirmation: ProposalConfirmation,
    requested_budget: ProposalBudget,
    cost: ProposalCost,
}

impl TipReplacementProposal {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn parent_revision(&self) -> u64 {
        self.parent_revision
    }

    #[must_use]
    pub fn parent_digest(&self) -> &str {
        &self.parent_digest
    }

    #[must_use]
    pub const fn superseded_revision(&self) -> u64 {
        self.superseded_revision
    }

    #[must_use]
    pub fn superseded_digest(&self) -> &str {
        &self.superseded_digest
    }

    #[must_use]
    pub const fn corrected_revision(&self) -> u64 {
        self.corrected_revision
    }

    #[must_use]
    pub const fn batch(&self) -> &CommandBatch {
        &self.batch
    }

    #[must_use]
    pub fn command_digest(&self) -> &str {
        &self.command_digest
    }

    #[must_use]
    pub fn dependency_digest(&self) -> &str {
        &self.dependency_digest
    }

    #[must_use]
    pub const fn authoritative_dependencies(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_dependencies
    }

    #[must_use]
    pub const fn authoritative_writes(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_writes
    }

    #[must_use]
    pub fn authoritative_diff(&self) -> &[ProposalDiffEntry] {
        &self.authoritative_diff
    }

    #[must_use]
    pub fn intended_result_digest(&self) -> &str {
        &self.intended_result_digest
    }

    #[must_use]
    pub const fn principal(&self) -> ProposalPrincipal {
        self.principal
    }

    #[must_use]
    pub fn goal(&self) -> ProposalGoal {
        self.goal.clone()
    }

    #[must_use]
    pub fn assumptions(&self) -> &[ProposalAssumption] {
        &self.assumptions
    }

    #[must_use]
    pub const fn risk(&self) -> ProposalRisk {
        self.risk
    }

    #[must_use]
    pub const fn confirmation(&self) -> &ProposalConfirmation {
        &self.confirmation
    }

    #[must_use]
    pub const fn requested_budget(&self) -> ProposalBudget {
        self.requested_budget
    }

    #[must_use]
    pub const fn cost(&self) -> ProposalCost {
        self.cost
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    document_id: DocumentId,
    provenance_revision: u64,
    provenance_digest: String,
    batch: CommandBatch,
    command_digest: String,
    authoritative_dependencies: BTreeSet<AuthoritativeDependency>,
    authoritative_writes: BTreeSet<AuthoritativeDependency>,
    dependency_digest: String,
    authoritative_diff: Vec<ProposalDiffEntry>,
    intended_result_digest: String,
    principal: ProposalPrincipal,
    goal: ProposalGoal,
    assumptions: Vec<ProposalAssumption>,
    risk: ProposalRisk,
    confirmation: ProposalConfirmation,
    requested_budget: ProposalBudget,
    cost: ProposalCost,
}

impl Proposal {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn provenance_revision(&self) -> u64 {
        self.provenance_revision
    }

    #[must_use]
    pub fn provenance_digest(&self) -> &str {
        &self.provenance_digest
    }

    #[must_use]
    pub const fn batch(&self) -> &CommandBatch {
        &self.batch
    }

    #[must_use]
    pub fn command_digest(&self) -> &str {
        &self.command_digest
    }

    #[must_use]
    pub const fn authoritative_dependencies(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_dependencies
    }

    #[must_use]
    pub const fn authoritative_writes(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_writes
    }

    #[must_use]
    pub fn authoritative_diff(&self) -> &[ProposalDiffEntry] {
        &self.authoritative_diff
    }

    #[must_use]
    pub fn intended_result_digest(&self) -> &str {
        &self.intended_result_digest
    }

    #[must_use]
    pub const fn principal(&self) -> ProposalPrincipal {
        self.principal
    }

    #[must_use]
    pub fn goal(&self) -> ProposalGoal {
        self.goal.clone()
    }

    #[must_use]
    pub fn assumptions(&self) -> &[ProposalAssumption] {
        &self.assumptions
    }

    #[must_use]
    pub const fn risk(&self) -> ProposalRisk {
        self.risk
    }

    #[must_use]
    pub const fn confirmation(&self) -> &ProposalConfirmation {
        &self.confirmation
    }

    #[must_use]
    pub const fn requested_budget(&self) -> ProposalBudget {
        self.requested_budget
    }

    #[must_use]
    pub const fn cost(&self) -> ProposalCost {
        self.cost
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideEffectProposal {
    document_id: DocumentId,
    provenance_revision: u64,
    provenance_digest: String,
    operation: String,
    operation_digest: String,
    payload_digest: String,
    principal: ProposalPrincipal,
    scope: HighRiskScope,
}

impl SideEffectProposal {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn provenance_revision(&self) -> u64 {
        self.provenance_revision
    }

    #[must_use]
    pub fn provenance_digest(&self) -> &str {
        &self.provenance_digest
    }

    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    #[must_use]
    pub fn operation_digest(&self) -> &str {
        &self.operation_digest
    }

    #[must_use]
    pub fn payload_digest(&self) -> &str {
        &self.payload_digest
    }

    #[must_use]
    pub const fn principal(&self) -> ProposalPrincipal {
        self.principal
    }

    #[must_use]
    pub const fn scope(&self) -> &HighRiskScope {
        &self.scope
    }
}

pub const MAX_HUMAN_CONFIRMATION_LIFETIME_MS: u64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthenticatedApprover {
    Human(u64),
    Machine(ProposalPrincipal),
}

pub struct TrustedConfirmationSurface {
    signing_key: SigningKey,
    policy_epoch: u64,
}

impl TrustedConfirmationSurface {
    pub fn new(signing_key: [u8; 32], policy_epoch: u64) -> Result<Self, HumanConfirmationError> {
        if policy_epoch == 0 {
            return Err(HumanConfirmationError::PolicyEpochInvalid);
        }
        Ok(Self {
            signing_key: SigningKey::from_bytes(&signing_key),
            policy_epoch,
        })
    }

    #[must_use]
    pub fn verifying_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn issue(
        &self,
        proposal: &Proposal,
        approver: AuthenticatedApprover,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<HumanApprovalToken, HumanConfirmationError> {
        let approving_human = match approver {
            AuthenticatedApprover::Human(id) if id != 0 => id,
            AuthenticatedApprover::Human(_) => {
                return Err(HumanConfirmationError::InvalidHumanPrincipal);
            }
            AuthenticatedApprover::Machine(_) => {
                return Err(HumanConfirmationError::MachineCannotApprove);
            }
        };
        if proposal.principal == ProposalPrincipal::Human(approving_human) {
            return Err(HumanConfirmationError::RequesterCannotApprove);
        }
        let ProposalRisk::High(risk_class) = proposal.risk else {
            return Err(HumanConfirmationError::NotHighRisk);
        };
        let ProposalConfirmation::HumanOnly(scope) = &proposal.confirmation else {
            return Err(HumanConfirmationError::ConfirmationRequirementMismatch);
        };
        if scope.class != risk_class {
            return Err(HumanConfirmationError::ConfirmationRequirementMismatch);
        }
        if expires_at_ms <= issued_at_ms
            || expires_at_ms - issued_at_ms > MAX_HUMAN_CONFIRMATION_LIFETIME_MS
        {
            return Err(HumanConfirmationError::InvalidLifetime);
        }
        let mut token = HumanApprovalToken {
            requester: proposal.principal,
            approving_human,
            document_id: proposal.document_id,
            revision_id: proposal.provenance_revision,
            provenance_digest: proposal.provenance_digest.clone(),
            dependency_digest: proposal.dependency_digest.clone(),
            command_digest: proposal.command_digest.clone(),
            result_digest: proposal.intended_result_digest.clone(),
            scope: scope.clone(),
            policy_epoch: self.policy_epoch,
            issued_at_ms,
            expires_at_ms,
            signature: [0; 64],
        };
        token.signature = self.signing_key.sign(&token.signing_payload()).to_bytes();
        Ok(token)
    }

    pub fn issue_side_effect(
        &self,
        proposal: &SideEffectProposal,
        approver: AuthenticatedApprover,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<SideEffectApprovalToken, HumanConfirmationError> {
        let approving_human = authenticated_human(approver)?;
        if proposal.principal == ProposalPrincipal::Human(approving_human) {
            return Err(HumanConfirmationError::RequesterCannotApprove);
        }
        validate_confirmation_lifetime(issued_at_ms, expires_at_ms)?;
        let mut token = SideEffectApprovalToken {
            requester: proposal.principal,
            approving_human,
            document_id: proposal.document_id,
            revision_id: proposal.provenance_revision,
            provenance_digest: proposal.provenance_digest.clone(),
            operation_digest: proposal.operation_digest.clone(),
            payload_digest: proposal.payload_digest.clone(),
            scope: proposal.scope.clone(),
            policy_epoch: self.policy_epoch,
            issued_at_ms,
            expires_at_ms,
            signature: [0; 64],
        };
        token.signature = self.signing_key.sign(&token.signing_payload()).to_bytes();
        Ok(token)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanApprovalToken {
    requester: ProposalPrincipal,
    approving_human: u64,
    document_id: DocumentId,
    revision_id: u64,
    provenance_digest: String,
    dependency_digest: String,
    command_digest: String,
    result_digest: String,
    scope: HighRiskScope,
    policy_epoch: u64,
    issued_at_ms: u64,
    expires_at_ms: u64,
    signature: [u8; 64],
}

impl HumanApprovalToken {
    #[must_use]
    pub const fn approving_human(&self) -> u64 {
        self.approving_human
    }

    #[must_use]
    pub const fn policy_epoch(&self) -> u64 {
        self.policy_epoch
    }

    #[must_use]
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    fn signing_payload(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_confirmation_field(&mut bytes, b"ketchup.human-confirmation.v1");
        push_confirmation_principal(&mut bytes, self.requester);
        push_confirmation_u64(&mut bytes, self.approving_human);
        push_confirmation_u64(&mut bytes, self.document_id.0);
        push_confirmation_u64(&mut bytes, self.revision_id);
        push_confirmation_field(&mut bytes, self.provenance_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.dependency_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.command_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.result_digest.as_bytes());
        push_confirmation_scope(&mut bytes, &self.scope);
        push_confirmation_u64(&mut bytes, self.policy_epoch);
        push_confirmation_u64(&mut bytes, self.issued_at_ms);
        push_confirmation_u64(&mut bytes, self.expires_at_ms);
        bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideEffectApprovalToken {
    requester: ProposalPrincipal,
    approving_human: u64,
    document_id: DocumentId,
    revision_id: u64,
    provenance_digest: String,
    operation_digest: String,
    payload_digest: String,
    scope: HighRiskScope,
    policy_epoch: u64,
    issued_at_ms: u64,
    expires_at_ms: u64,
    signature: [u8; 64],
}

impl SideEffectApprovalToken {
    fn signing_payload(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_confirmation_field(&mut bytes, b"ketchup.side-effect-confirmation.v1");
        push_confirmation_principal(&mut bytes, self.requester);
        push_confirmation_u64(&mut bytes, self.approving_human);
        push_confirmation_u64(&mut bytes, self.document_id.0);
        push_confirmation_u64(&mut bytes, self.revision_id);
        push_confirmation_field(&mut bytes, self.provenance_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.operation_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.payload_digest.as_bytes());
        push_confirmation_scope(&mut bytes, &self.scope);
        push_confirmation_u64(&mut bytes, self.policy_epoch);
        push_confirmation_u64(&mut bytes, self.issued_at_ms);
        push_confirmation_u64(&mut bytes, self.expires_at_ms);
        bytes
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SideEffectAuthorizationReceipt {
    approving_human: u64,
    document_id: DocumentId,
    revision_id: u64,
    operation: String,
    operation_digest: String,
    payload_digest: String,
    scope: HighRiskScope,
    policy_epoch: u64,
    authorized_at_ms: u64,
}

impl SideEffectAuthorizationReceipt {
    #[must_use]
    pub const fn approving_human(&self) -> u64 {
        self.approving_human
    }

    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn revision_id(&self) -> u64 {
        self.revision_id
    }

    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    #[must_use]
    pub fn operation_digest(&self) -> &str {
        &self.operation_digest
    }

    #[must_use]
    pub fn payload_digest(&self) -> &str {
        &self.payload_digest
    }

    #[must_use]
    pub const fn scope(&self) -> &HighRiskScope {
        &self.scope
    }

    #[must_use]
    pub const fn policy_epoch(&self) -> u64 {
        self.policy_epoch
    }

    #[must_use]
    pub const fn authorized_at_ms(&self) -> u64 {
        self.authorized_at_ms
    }
}

#[derive(Clone)]
pub struct VerifiedProposalCommit {
    revision: Arc<Revision>,
    command_digest: String,
    result_digest: String,
    verified_writes: BTreeSet<AuthoritativeDependency>,
}

impl VerifiedProposalCommit {
    #[must_use]
    pub const fn revision(&self) -> &Arc<Revision> {
        &self.revision
    }

    #[must_use]
    pub fn command_digest(&self) -> &str {
        &self.command_digest
    }

    #[must_use]
    pub fn result_digest(&self) -> &str {
        &self.result_digest
    }

    #[must_use]
    pub const fn verified_writes(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.verified_writes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalValidity {
    Valid {
        evaluated_revision: u64,
    },
    Stale {
        provenance_revision: u64,
        current_revision: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HumanConfirmationError {
    InvalidScope,
    PolicyEpochInvalid,
    InvalidVerifyingKey,
    InvalidHumanPrincipal,
    UnidentifiedRequester,
    MachineCannotApprove,
    RequesterCannotApprove,
    NotHighRisk,
    ConfirmationRequirementMismatch,
    InvalidLifetime,
    InvalidSideEffectEvidence,
}

impl fmt::Display for HumanConfirmationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScope => formatter.write_str("high-risk confirmation scope is invalid"),
            Self::PolicyEpochInvalid => {
                formatter.write_str("human-confirmation policy epoch must advance from non-zero")
            }
            Self::InvalidVerifyingKey => {
                formatter.write_str("human-confirmation verifying key is invalid")
            }
            Self::InvalidHumanPrincipal => {
                formatter.write_str("approving human principal must be authenticated and non-zero")
            }
            Self::UnidentifiedRequester => formatter.write_str(
                "high-risk proposal requires an explicitly identified requesting principal",
            ),
            Self::MachineCannotApprove => {
                formatter.write_str("machine principals cannot approve human-only operations")
            }
            Self::RequesterCannotApprove => {
                formatter.write_str("requesting human cannot satisfy distinct-human approval")
            }
            Self::NotHighRisk => {
                formatter.write_str("human-only approval applies only to high-risk proposals")
            }
            Self::ConfirmationRequirementMismatch => {
                formatter.write_str("proposal risk and confirmation requirement do not match")
            }
            Self::InvalidLifetime => formatter
                .write_str("human confirmation lifetime must be positive and at most five minutes"),
            Self::InvalidSideEffectEvidence => formatter.write_str(
                "side-effect operation and payload evidence must be non-empty and bounded",
            ),
        }
    }
}

impl std::error::Error for HumanConfirmationError {}

#[derive(Debug, PartialEq)]
pub enum ProposalPrepareError {
    HostBudgetExceeded,
    RequestedBudgetExceeded,
    Confirmation(HumanConfirmationError),
    Canonical(CanonicalError),
}

impl fmt::Display for ProposalPrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostBudgetExceeded => formatter.write_str("proposal budget exceeds host policy"),
            Self::RequestedBudgetExceeded => {
                formatter.write_str("proposal work exceeds its requested budget")
            }
            Self::Confirmation(error) => error.fmt(formatter),
            Self::Canonical(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProposalPrepareError {}

impl From<CanonicalError> for ProposalPrepareError {
    fn from(error: CanonicalError) -> Self {
        Self::Canonical(error)
    }
}

#[derive(Debug, PartialEq)]
pub enum TipReplacementProposalError {
    NoCurrentTip,
    RedoBranch,
    Stale,
    Preparation(ProposalPrepareError),
    Canonical(CanonicalError),
    VerificationMismatch,
    HumanApprovalRequired,
}

impl fmt::Display for TipReplacementProposalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCurrentTip => {
                formatter.write_str("tip replacement requires an immediate parent revision")
            }
            Self::RedoBranch => {
                formatter.write_str("tip replacement requires the current last revision")
            }
            Self::Stale => formatter.write_str("tip replacement provenance is stale"),
            Self::Preparation(error) => error.fmt(formatter),
            Self::Canonical(error) => error.fmt(formatter),
            Self::VerificationMismatch => {
                formatter.write_str("tip replacement proposal verification failed")
            }
            Self::HumanApprovalRequired => formatter
                .write_str("high-risk tip replacement requires authenticated human approval"),
        }
    }
}

impl std::error::Error for TipReplacementProposalError {}

#[derive(Debug, PartialEq)]
pub enum ProposalCommitError {
    Stale(ProposalValidity),
    Preparation(ProposalPrepareError),
    Canonical(CanonicalError),
    VerificationMismatch,
    HumanApprovalRequired,
    HumanApprovalUnexpected,
    HumanApprovalPolicyUnavailable,
    HumanApprovalPolicyStale,
    HumanApprovalInvalid,
    HumanApprovalReplayed,
}

impl fmt::Display for ProposalCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale(_) => formatter.write_str("proposal dependencies changed"),
            Self::Preparation(error) => error.fmt(formatter),
            Self::Canonical(error) => error.fmt(formatter),
            Self::VerificationMismatch => formatter.write_str("proposal verification failed"),
            Self::HumanApprovalRequired => {
                formatter.write_str("high-risk proposal requires authenticated human approval")
            }
            Self::HumanApprovalUnexpected => {
                formatter.write_str("human-only approval was supplied for a standard proposal")
            }
            Self::HumanApprovalPolicyUnavailable => {
                formatter.write_str("trusted human-confirmation policy is unavailable")
            }
            Self::HumanApprovalPolicyStale => {
                formatter.write_str("human-confirmation policy epoch is stale")
            }
            Self::HumanApprovalInvalid => {
                formatter.write_str("human confirmation is invalid, expired, or mismatched")
            }
            Self::HumanApprovalReplayed => {
                formatter.write_str("human confirmation was already consumed")
            }
        }
    }
}

impl std::error::Error for ProposalCommitError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideEffectAuthorizationError {
    PolicyUnavailable,
    PolicyStale,
    Invalid,
    Replayed,
}

impl fmt::Display for SideEffectAuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PolicyUnavailable => {
                formatter.write_str("trusted side-effect confirmation policy is unavailable")
            }
            Self::PolicyStale => formatter.write_str("side-effect confirmation policy is stale"),
            Self::Invalid => formatter
                .write_str("side-effect confirmation is invalid, expired, stale, or mismatched"),
            Self::Replayed => formatter.write_str("side-effect confirmation was already consumed"),
        }
    }
}

impl std::error::Error for SideEffectAuthorizationError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalError {
    EmptySourceToken,
    InvalidDecimalToken,
    DimensionOutsideEnvelope,
    InvalidRevolve,
    InvalidPlanarOffset,
    InvalidSweep,
    InvalidSplineProfile,
    InvalidLoft,
    ReservedNodeId,
    EmptyNodeName,
    DependenciesNotCanonical,
    DependencyCycle(NodeId),
    NodeAlreadyExists(NodeId),
    NodeNotFound(NodeId),
    MissingDependency(NodeId),
    UnsupportedCommandSchema,
    EmptyCommandBatch,
    ReservedProductId,
    EmptyProductName,
    InvalidTransform,
    InvalidProfile,
    Sketch(SketchError),
    InvalidStableSubshapeRole,
    SubshapeRolesNotCanonical,
    InvalidTopologicalFeatureReference,
    InvalidMeshBody,
    DefinitionAlreadyExists(DefinitionId),
    DefinitionNotFound(DefinitionId),
    DefinitionInUse(DefinitionId),
    DefinitionNotEmpty(DefinitionId),
    BodyAlreadyExists(DefinitionId, BodyId),
    BodyNotFound(DefinitionId, BodyId),
    BodyInUse(DefinitionId, BodyId),
    BodyIsActive(DefinitionId, BodyId),
    BodyInputsNotCanonical,
    InvalidBodyCommand,
    InvalidBodyAuthoringPlan,
    InvalidBodyContract,
    InvalidBodyOwnership(FeatureId),
    BodyDependencyCycle(DefinitionId),
    UnresolvedBodyOwnershipReference(FeatureId),
    FeatureAlreadyExists(FeatureId),
    FeatureNotFound(FeatureId),
    FeatureHasNoDimension(FeatureId),
    FeatureIsNotProfile(FeatureId),
    FeatureDependencyCycle(FeatureId),
    InvalidFeatureSuppression(DefinitionId, BodyId),
    FeatureSuppressionUnchanged(DefinitionId, BodyId),
    InvalidFeatureParameterBinding(FeatureParameterTarget),
    FeatureParameterBindingNotFound(FeatureParameterTarget),
    OccurrenceAlreadyExists(OccurrenceId),
    OccurrenceNotFound(OccurrenceId),
    OccurrenceInAssemblyMate(OccurrenceId),
    OccurrenceInAssemblyJoint(OccurrenceId),
    AssemblyMateAlreadyExists(AssemblyMateId),
    AssemblyMateNotFound(AssemblyMateId),
    InvalidAssemblyMate(AssemblyMateId),
    AssemblyJointAlreadyExists(AssemblyJointId),
    AssemblyJointNotFound(AssemblyJointId),
    AssemblyJointInMotionStudy(AssemblyJointId),
    AssemblyJointInMotionCoupling(AssemblyJointId),
    InvalidAssemblyJoint(AssemblyJointId),
    AssemblyMotionCouplingAlreadyExists(AssemblyMotionCouplingId),
    AssemblyMotionCouplingNotFound(AssemblyMotionCouplingId),
    InvalidAssemblyMotionCoupling(AssemblyMotionCouplingId),
    UnsynchronizedAssemblyJointPosition(AssemblyJointId),
    AssemblyMotionStudyAlreadyExists(AssemblyMotionStudyId),
    AssemblyMotionStudyNotFound(AssemblyMotionStudyId),
    InvalidAssemblyMotionStudy(AssemblyMotionStudyId),
    StaleAssemblySolve,
    InvalidAssemblySolvePublication,
    DrawingSheetAlreadyExists(DrawingSheetId),
    DrawingSheetNotFound(DrawingSheetId),
    Drawing(DrawingError),
    GroupAlreadyExists(GroupId),
    GroupNotFound(GroupId),
    GroupNotEmpty(GroupId),
    GroupCycle(GroupId),
    InvalidFeatureOwnership(FeatureId),
    InvalidFeatureMap,
    InvalidSolidToolPlan,
    UnsupportedSolidToolTransform,
    OccurrenceDefinitionMismatch,
    InvalidLocalGraph,
    InvalidInstancePath,
    IdExhausted,
    WrongNodeKind(NodeId),
    OverrideAlreadyExists(u64),
    OverrideNotFound(u64),
    JointAlreadyExists(JointId),
    JointNotFound(JointId),
    SpaceAlreadyExists(SpaceId),
    SpaceNotFound(SpaceId),
    ClearanceVolumeAlreadyExists(ClearanceVolumeId),
    ClearanceVolumeNotFound(ClearanceVolumeId),
    PersistentDimensionNotFound(PersistentDimensionId),
    PersistentDimensionAlreadyExists(PersistentDimensionId),
    TagAlreadyExists(TagId),
    TagNotFound(TagId),
    TagInUse(TagId),
    InvalidClassificationDimension(ClassificationDimensionId),
    ClassificationDimensionNotFound(ClassificationDimensionId),
    ClassificationCategoryNotFound(ClassificationDimensionId, ClassificationCategoryId),
    ClassificationCategoryInUse(ClassificationDimensionId),
    CollectionAlreadyExists(CollectionId),
    CollectionNotFound(CollectionId),
    CollectionMembershipNotCanonical(CollectionId),
    OccurrenceInCollection(OccurrenceId),
    InvalidImportReceipt,
    ImportAlreadyExists(ImportId),
    InvalidPersistentDimensionTarget,
    InvalidDimensionPresentation,
    UndeclaredOverrideParameter,
    UnresolvedDerivedOutput,
    EvaluationEnvelopeMismatch,
    EvaluationEvidenceMismatch,
    FailedEvaluation(NodeId),
    RevisionExhausted,
    Graph(GraphError),
    Prismatic(PrismaticError),
    Space(SpaceError),
}

impl CanonicalError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::EmptySourceToken => "canonical.empty_source_token",
            Self::InvalidDecimalToken => "canonical.invalid_decimal_token",
            Self::DimensionOutsideEnvelope => "canonical.dimension_outside_envelope",
            Self::InvalidRevolve => "canonical.invalid_revolve",
            Self::InvalidPlanarOffset => "canonical.invalid_planar_offset",
            Self::InvalidSweep => "canonical.invalid_sweep",
            Self::InvalidSplineProfile => "canonical.invalid_spline_profile",
            Self::InvalidLoft => "canonical.invalid_loft",
            Self::ReservedNodeId => "canonical.reserved_node_id",
            Self::EmptyNodeName => "canonical.empty_node_name",
            Self::DependenciesNotCanonical => "canonical.dependencies_not_canonical",
            Self::DependencyCycle(..) => "canonical.dependency_cycle",
            Self::NodeAlreadyExists(..) => "canonical.node_already_exists",
            Self::NodeNotFound(..) => "canonical.node_not_found",
            Self::MissingDependency(..) => "canonical.missing_dependency",
            Self::UnsupportedCommandSchema => "canonical.unsupported_command_schema",
            Self::EmptyCommandBatch => "canonical.empty_command_batch",
            Self::ReservedProductId => "canonical.reserved_product_id",
            Self::EmptyProductName => "canonical.empty_product_name",
            Self::InvalidTransform => "canonical.invalid_transform",
            Self::InvalidProfile => "canonical.invalid_profile",
            Self::Sketch(..) => "canonical.sketch",
            Self::InvalidStableSubshapeRole => "canonical.invalid_stable_subshape_role",
            Self::SubshapeRolesNotCanonical => "canonical.subshape_roles_not_canonical",
            Self::InvalidTopologicalFeatureReference => {
                "canonical.invalid_topological_feature_reference"
            }
            Self::InvalidMeshBody => "canonical.invalid_mesh_body",
            Self::DefinitionAlreadyExists(..) => "canonical.definition_already_exists",
            Self::DefinitionNotFound(..) => "canonical.definition_not_found",
            Self::DefinitionInUse(..) => "canonical.definition_in_use",
            Self::DefinitionNotEmpty(..) => "canonical.definition_not_empty",
            Self::BodyAlreadyExists(..) => "canonical.body_already_exists",
            Self::BodyNotFound(..) => "canonical.body_not_found",
            Self::BodyInUse(..) => "canonical.body_in_use",
            Self::BodyIsActive(..) => "canonical.body_is_active",
            Self::BodyInputsNotCanonical => "canonical.body_inputs_not_canonical",
            Self::InvalidBodyCommand => "canonical.invalid_body_command",
            Self::InvalidBodyAuthoringPlan => "canonical.invalid_body_authoring_plan",
            Self::InvalidBodyContract => "canonical.invalid_body_contract",
            Self::InvalidBodyOwnership(..) => "canonical.invalid_body_ownership",
            Self::BodyDependencyCycle(..) => "canonical.body_dependency_cycle",
            Self::UnresolvedBodyOwnershipReference(..) => {
                "canonical.unresolved_body_ownership_reference"
            }
            Self::FeatureAlreadyExists(..) => "canonical.feature_already_exists",
            Self::FeatureNotFound(..) => "canonical.feature_not_found",
            Self::FeatureHasNoDimension(..) => "canonical.feature_has_no_dimension",
            Self::FeatureIsNotProfile(..) => "canonical.feature_is_not_profile",
            Self::FeatureDependencyCycle(..) => "canonical.feature_dependency_cycle",
            Self::InvalidFeatureSuppression(..) => "canonical.invalid_feature_suppression",
            Self::FeatureSuppressionUnchanged(..) => "canonical.feature_suppression_unchanged",
            Self::InvalidFeatureParameterBinding(..) => {
                "canonical.invalid_feature_parameter_binding"
            }
            Self::FeatureParameterBindingNotFound(..) => {
                "canonical.feature_parameter_binding_not_found"
            }
            Self::OccurrenceAlreadyExists(..) => "canonical.occurrence_already_exists",
            Self::OccurrenceNotFound(..) => "canonical.occurrence_not_found",
            Self::OccurrenceInAssemblyMate(..) => "canonical.occurrence_in_assembly_mate",
            Self::OccurrenceInAssemblyJoint(..) => "canonical.occurrence_in_assembly_joint",
            Self::AssemblyMateAlreadyExists(..) => "canonical.assembly_mate_already_exists",
            Self::AssemblyMateNotFound(..) => "canonical.assembly_mate_not_found",
            Self::InvalidAssemblyMate(..) => "canonical.invalid_assembly_mate",
            Self::AssemblyJointAlreadyExists(..) => "canonical.assembly_joint_already_exists",
            Self::AssemblyJointNotFound(..) => "canonical.assembly_joint_not_found",
            Self::AssemblyJointInMotionStudy(..) => "canonical.assembly_joint_in_motion_study",
            Self::AssemblyJointInMotionCoupling(..) => {
                "canonical.assembly_joint_in_motion_coupling"
            }
            Self::InvalidAssemblyJoint(..) => "canonical.invalid_assembly_joint",
            Self::AssemblyMotionCouplingAlreadyExists(..) => {
                "canonical.assembly_motion_coupling_already_exists"
            }
            Self::AssemblyMotionCouplingNotFound(..) => {
                "canonical.assembly_motion_coupling_not_found"
            }
            Self::InvalidAssemblyMotionCoupling(..) => "canonical.invalid_assembly_motion_coupling",
            Self::UnsynchronizedAssemblyJointPosition(..) => {
                "canonical.unsynchronized_assembly_joint_position"
            }
            Self::AssemblyMotionStudyAlreadyExists(..) => {
                "canonical.assembly_motion_study_already_exists"
            }
            Self::AssemblyMotionStudyNotFound(..) => "canonical.assembly_motion_study_not_found",
            Self::InvalidAssemblyMotionStudy(..) => "canonical.invalid_assembly_motion_study",
            Self::StaleAssemblySolve => "canonical.stale_assembly_solve",
            Self::InvalidAssemblySolvePublication => "canonical.invalid_assembly_solve_publication",
            Self::DrawingSheetAlreadyExists(..) => "canonical.drawing_sheet_already_exists",
            Self::DrawingSheetNotFound(..) => "canonical.drawing_sheet_not_found",
            Self::Drawing(..) => "canonical.drawing",
            Self::GroupAlreadyExists(..) => "canonical.group_already_exists",
            Self::GroupNotFound(..) => "canonical.group_not_found",
            Self::GroupNotEmpty(..) => "canonical.group_not_empty",
            Self::GroupCycle(..) => "canonical.group_cycle",
            Self::InvalidFeatureOwnership(..) => "canonical.invalid_feature_ownership",
            Self::InvalidFeatureMap => "canonical.invalid_feature_map",
            Self::InvalidSolidToolPlan => "canonical.invalid_solid_tool_plan",
            Self::UnsupportedSolidToolTransform => "canonical.unsupported_solid_tool_transform",
            Self::OccurrenceDefinitionMismatch => "canonical.occurrence_definition_mismatch",
            Self::InvalidLocalGraph => "canonical.invalid_local_graph",
            Self::InvalidInstancePath => "canonical.invalid_instance_path",
            Self::IdExhausted => "canonical.id_exhausted",
            Self::WrongNodeKind(..) => "canonical.wrong_node_kind",
            Self::OverrideAlreadyExists(..) => "canonical.override_already_exists",
            Self::OverrideNotFound(..) => "canonical.override_not_found",
            Self::JointAlreadyExists(..) => "canonical.joint_already_exists",
            Self::JointNotFound(..) => "canonical.joint_not_found",
            Self::SpaceAlreadyExists(..) => "canonical.space_already_exists",
            Self::SpaceNotFound(..) => "canonical.space_not_found",
            Self::ClearanceVolumeAlreadyExists(..) => "canonical.clearance_volume_already_exists",
            Self::ClearanceVolumeNotFound(..) => "canonical.clearance_volume_not_found",
            Self::PersistentDimensionNotFound(..) => "canonical.persistent_dimension_not_found",
            Self::PersistentDimensionAlreadyExists(..) => {
                "canonical.persistent_dimension_already_exists"
            }
            Self::TagAlreadyExists(..) => "canonical.tag_already_exists",
            Self::TagNotFound(..) => "canonical.tag_not_found",
            Self::TagInUse(..) => "canonical.tag_in_use",
            Self::InvalidClassificationDimension(..) => {
                "canonical.invalid_classification_dimension"
            }
            Self::ClassificationDimensionNotFound(..) => {
                "canonical.classification_dimension_not_found"
            }
            Self::ClassificationCategoryNotFound(..) => {
                "canonical.classification_category_not_found"
            }
            Self::ClassificationCategoryInUse(..) => "canonical.classification_category_in_use",
            Self::CollectionAlreadyExists(..) => "canonical.collection_already_exists",
            Self::CollectionNotFound(..) => "canonical.collection_not_found",
            Self::CollectionMembershipNotCanonical(..) => {
                "canonical.collection_membership_not_canonical"
            }
            Self::OccurrenceInCollection(..) => "canonical.occurrence_in_collection",
            Self::InvalidImportReceipt => "canonical.invalid_import_receipt",
            Self::ImportAlreadyExists(..) => "canonical.import_already_exists",
            Self::InvalidPersistentDimensionTarget => {
                "canonical.invalid_persistent_dimension_target"
            }
            Self::InvalidDimensionPresentation => "canonical.invalid_dimension_presentation",
            Self::UndeclaredOverrideParameter => "canonical.undeclared_override_parameter",
            Self::UnresolvedDerivedOutput => "canonical.unresolved_derived_output",
            Self::EvaluationEnvelopeMismatch => "canonical.evaluation_envelope_mismatch",
            Self::EvaluationEvidenceMismatch => "canonical.evaluation_evidence_mismatch",
            Self::FailedEvaluation(..) => "canonical.failed_evaluation",
            Self::RevisionExhausted => "canonical.revision_exhausted",
            Self::Graph(..) => "canonical.graph",
            Self::Prismatic(..) => "canonical.prismatic",
            Self::Space(..) => "canonical.space",
        }
    }
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySourceToken => formatter.write_str("dimension source token is empty"),
            Self::InvalidDecimalToken => {
                formatter.write_str("dimension source token is not decimal")
            }
            Self::DimensionOutsideEnvelope => {
                formatter.write_str("dimension is outside the canonical coordinate envelope")
            }
            Self::InvalidRevolve => formatter.write_str("revolve axis or angle is invalid"),
            Self::InvalidPlanarOffset => {
                formatter.write_str("planar offset distance or bounded profile is invalid")
            }
            Self::InvalidSweep => {
                formatter.write_str("sweep requires a bounded profile and straight open path")
            }
            Self::InvalidSplineProfile => {
                formatter.write_str("spline profile requires bounded canonical control points")
            }
            Self::InvalidLoft => {
                formatter.write_str("loft requires ordered bounded spline-profile sections")
            }
            Self::ReservedNodeId => formatter.write_str("node ID zero is reserved"),
            Self::EmptyNodeName => formatter.write_str("node name is empty"),
            Self::DependenciesNotCanonical => {
                formatter.write_str("dependencies must be unique and strictly sorted")
            }
            Self::DependencyCycle(id) => write!(formatter, "dependency cycle at node {}", id.0),
            Self::NodeAlreadyExists(id) => write!(formatter, "node {} already exists", id.0),
            Self::NodeNotFound(id) => write!(formatter, "node {} does not exist", id.0),
            Self::MissingDependency(id) => write!(formatter, "dependency {} does not exist", id.0),
            Self::UnsupportedCommandSchema => formatter.write_str("unsupported command schema"),
            Self::EmptyCommandBatch => formatter.write_str("command batch is empty"),
            Self::ReservedProductId => formatter.write_str("product entity ID zero is reserved"),
            Self::EmptyProductName => formatter.write_str("product entity name is empty"),
            Self::InvalidTransform => {
                formatter.write_str("transform is not a finite affine matrix")
            }
            Self::InvalidProfile => {
                formatter.write_str("profile must contain finite non-degenerate points")
            }
            Self::Sketch(error) => write!(formatter, "invalid workplane or sketch: {error}"),
            Self::InvalidStableSubshapeRole => formatter.write_str(
                "stable subshape role must be a bounded canonical semantic identifier",
            ),
            Self::SubshapeRolesNotCanonical => formatter.write_str(
                "stable subshape roles must be non-empty, unique, and strictly sorted",
            ),
            Self::InvalidTopologicalFeatureReference => formatter.write_str(
                "topological feature references must be valid, kind-correct, unique, and strictly sorted",
            ),
            Self::InvalidMeshBody => formatter.write_str(
                "mesh body must be finite, closed, consistently oriented, non-degenerate, and carry valid authority provenance",
            ),
            Self::DefinitionAlreadyExists(id) => {
                write!(formatter, "definition {} already exists", id.0)
            }
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} does not exist", id.0),
            Self::DefinitionInUse(id) => write!(formatter, "definition {} is still used", id.0),
            Self::DefinitionNotEmpty(id) => write!(formatter, "definition {} is not empty", id.0),
            Self::BodyAlreadyExists(definition, body) => write!(
                formatter,
                "body {} already exists in definition {}",
                body.0, definition.0
            ),
            Self::BodyNotFound(definition, body) => write!(
                formatter,
                "body {} does not exist in definition {}",
                body.0, definition.0
            ),
            Self::BodyInUse(definition, body) => write!(
                formatter,
                "body {} in definition {} is still used",
                body.0, definition.0
            ),
            Self::BodyIsActive(definition, body) => write!(
                formatter,
                "body {} in definition {} is active",
                body.0, definition.0
            ),
            Self::BodyInputsNotCanonical => formatter
                .write_str("feature input bodies must be unique and strictly sorted"),
            Self::InvalidBodyCommand => formatter.write_str("command is not a body mutation"),
            Self::InvalidBodyAuthoringPlan => formatter.write_str(
                "multi-body authoring requires distinct bodies with resolved terminal Pad or Extrusion outputs",
            ),
            Self::InvalidBodyContract => {
                formatter.write_str("definition body contract is incomplete or non-canonical")
            }
            Self::InvalidBodyOwnership(id) => {
                write!(formatter, "feature {} has invalid body ownership", id.0)
            }
            Self::BodyDependencyCycle(id) => {
                write!(formatter, "body dependency cycle in definition {}", id.0)
            }
            Self::UnresolvedBodyOwnershipReference(id) => write!(
                formatter,
                "feature {} body ownership depends on an ambiguous or lost reference",
                id.0
            ),
            Self::FeatureAlreadyExists(id) => write!(formatter, "feature {} already exists", id.0),
            Self::FeatureNotFound(id) => write!(formatter, "feature {} does not exist", id.0),
            Self::FeatureHasNoDimension(id) => {
                write!(formatter, "feature {} has no editable dimension", id.0)
            }
            Self::FeatureIsNotProfile(id) => {
                write!(formatter, "feature {} is not a profile", id.0)
            }
            Self::FeatureDependencyCycle(id) => {
                write!(formatter, "feature dependency cycle at {}", id.0)
            }
            Self::InvalidFeatureSuppression(definition, body) => write!(
                formatter,
                "feature suppression for body {} in definition {} is not a dependency-closed suffix",
                body.0, definition.0
            ),
            Self::FeatureSuppressionUnchanged(definition, body) => write!(
                formatter,
                "feature suppression for body {} in definition {} is unchanged",
                body.0, definition.0
            ),
            Self::InvalidFeatureParameterBinding(target) => write!(
                formatter,
                "feature {} parameter {} has an invalid derived binding",
                target.feature_id.0,
                target.path.as_str()
            ),
            Self::FeatureParameterBindingNotFound(target) => write!(
                formatter,
                "feature {} parameter {} has no derived binding",
                target.feature_id.0,
                target.path.as_str()
            ),
            Self::OccurrenceAlreadyExists(id) => {
                write!(formatter, "occurrence {} already exists", id.0)
            }
            Self::OccurrenceNotFound(id) => write!(formatter, "occurrence {} does not exist", id.0),
            Self::OccurrenceInAssemblyMate(id) => {
                write!(formatter, "occurrence {} is still used by an assembly mate", id.0)
            }
            Self::OccurrenceInAssemblyJoint(id) => {
                write!(formatter, "occurrence {} is still used by an assembly joint", id.0)
            }
            Self::AssemblyMateAlreadyExists(id) => {
                write!(formatter, "assembly mate {} already exists", id.0)
            }
            Self::AssemblyMateNotFound(id) => {
                write!(formatter, "assembly mate {} does not exist", id.0)
            }
            Self::InvalidAssemblyMate(id) => {
                write!(formatter, "assembly mate {} is invalid or unresolved", id.0)
            }
            Self::AssemblyJointAlreadyExists(id) => {
                write!(formatter, "assembly joint {} already exists", id.0)
            }
            Self::AssemblyJointNotFound(id) => {
                write!(formatter, "assembly joint {} does not exist", id.0)
            }
            Self::AssemblyJointInMotionStudy(id) => {
                write!(formatter, "assembly joint {} is still used by a motion study", id.0)
            }
            Self::AssemblyJointInMotionCoupling(id) => write!(
                formatter,
                "assembly joint {} is still used by a motion coupling",
                id.0
            ),
            Self::InvalidAssemblyJoint(id) => {
                write!(formatter, "assembly joint {} is invalid", id.0)
            }
            Self::UnsynchronizedAssemblyJointPosition(id) => write!(
                formatter,
                "assembly joint {} position requires an atomic solve transform",
                id.0
            ),
            Self::AssemblyMotionCouplingAlreadyExists(id) => {
                write!(formatter, "assembly motion coupling {} already exists", id.0)
            }
            Self::AssemblyMotionCouplingNotFound(id) => {
                write!(formatter, "assembly motion coupling {} does not exist", id.0)
            }
            Self::InvalidAssemblyMotionCoupling(id) => {
                write!(formatter, "assembly motion coupling {} is invalid", id.0)
            }
            Self::AssemblyMotionStudyAlreadyExists(id) => {
                write!(formatter, "assembly motion study {} already exists", id.0)
            }
            Self::AssemblyMotionStudyNotFound(id) => {
                write!(formatter, "assembly motion study {} does not exist", id.0)
            }
            Self::InvalidAssemblyMotionStudy(id) => {
                write!(formatter, "assembly motion study {} is invalid", id.0)
            }
            Self::StaleAssemblySolve => {
                formatter.write_str("assembly solve source revision or digest is stale")
            }
            Self::InvalidAssemblySolvePublication => {
                formatter.write_str("assembly solve publication is empty, non-canonical, or grounded")
            }
            Self::DrawingSheetAlreadyExists(id) => {
                write!(formatter, "drawing sheet {} already exists", id.0)
            }
            Self::DrawingSheetNotFound(id) => {
                write!(formatter, "drawing sheet {} does not exist", id.0)
            }
            Self::Drawing(error) => write!(formatter, "invalid drawing sheet: {error}"),
            Self::GroupAlreadyExists(id) => write!(formatter, "group {} already exists", id.0),
            Self::GroupNotFound(id) => write!(formatter, "group {} does not exist", id.0),
            Self::GroupNotEmpty(id) => write!(formatter, "group {} is not empty", id.0),
            Self::GroupCycle(id) => write!(formatter, "group hierarchy cycle at {}", id.0),
            Self::InvalidFeatureOwnership(id) => write!(
                formatter,
                "feature {} has invalid definition ownership",
                id.0
            ),
            Self::InvalidFeatureMap => {
                formatter.write_str("feature clone map is incomplete or non-canonical")
            }
            Self::InvalidSolidToolPlan => {
                formatter.write_str("solid tool plan is incomplete or non-canonical")
            }
            Self::UnsupportedSolidToolTransform => formatter.write_str(
                "solid tools currently require root occurrences with translation-only transforms on the same extrusion plane",
            ),
            Self::OccurrenceDefinitionMismatch => {
                formatter.write_str("occurrence does not reference the requested source definition")
            }
            Self::InvalidLocalGraph => formatter.write_str("definition-local graph is invalid"),
            Self::InvalidInstancePath => formatter.write_str("instance path is unresolved"),
            Self::IdExhausted => formatter.write_str("canonical ID space is exhausted"),
            Self::WrongNodeKind(id) => write!(formatter, "node {} has the wrong kind", id.0),
            Self::OverrideAlreadyExists(id) => write!(formatter, "override {id} already exists"),
            Self::OverrideNotFound(id) => write!(formatter, "override {id} does not exist"),
            Self::JointAlreadyExists(id) => write!(formatter, "joint {} already exists", id.0),
            Self::JointNotFound(id) => write!(formatter, "joint {} does not exist", id.0),
            Self::SpaceAlreadyExists(id) => write!(formatter, "space {} already exists", id.0),
            Self::SpaceNotFound(id) => write!(formatter, "space {} does not exist", id.0),
            Self::ClearanceVolumeAlreadyExists(id) => {
                write!(formatter, "clearance volume {} already exists", id.0)
            }
            Self::ClearanceVolumeNotFound(id) => {
                write!(formatter, "clearance volume {} does not exist", id.0)
            }
            Self::PersistentDimensionNotFound(id) => {
                write!(formatter, "persistent dimension {} does not exist", id.0)
            }
            Self::PersistentDimensionAlreadyExists(id) => {
                write!(formatter, "persistent dimension {} already exists", id.0)
            }
            Self::TagAlreadyExists(id) => write!(formatter, "tag {} already exists", id.0),
            Self::TagNotFound(id) => write!(formatter, "tag {} does not exist", id.0),
            Self::TagInUse(id) => write!(formatter, "tag {} is still assigned", id.0),
            Self::InvalidClassificationDimension(id) => {
                write!(formatter, "classification dimension {} is invalid", id.0)
            }
            Self::ClassificationDimensionNotFound(id) => {
                write!(formatter, "classification dimension {} does not exist", id.0)
            }
            Self::ClassificationCategoryNotFound(dimension_id, category_id) => write!(
                formatter,
                "classification category {} does not exist in dimension {}",
                category_id.0, dimension_id.0
            ),
            Self::ClassificationCategoryInUse(id) => write!(
                formatter,
                "classification dimension {} would remove an assigned category",
                id.0
            ),
            Self::CollectionAlreadyExists(id) => {
                write!(formatter, "collection {} already exists", id.0)
            }
            Self::CollectionNotFound(id) => {
                write!(formatter, "collection {} does not exist", id.0)
            }
            Self::CollectionMembershipNotCanonical(id) => write!(
                formatter,
                "collection {} membership must be unique and strictly sorted",
                id.0
            ),
            Self::OccurrenceInCollection(id) => {
                write!(formatter, "occurrence {} is still in a collection", id.0)
            }
            Self::InvalidImportReceipt => formatter.write_str("import receipt is invalid"),
            Self::ImportAlreadyExists(id) => {
                write!(formatter, "import {} already exists", id.0)
            }
            Self::InvalidPersistentDimensionTarget => {
                formatter.write_str("persistent dimension target is invalid")
            }
            Self::InvalidDimensionPresentation => {
                formatter.write_str("persistent dimension presentation is invalid")
            }
            Self::UndeclaredOverrideParameter => {
                formatter.write_str("override parameter is not declared by the root rule")
            }
            Self::UnresolvedDerivedOutput => {
                formatter.write_str("derived output is unresolved or ambiguous")
            }
            Self::EvaluationEnvelopeMismatch => {
                formatter.write_str("evaluation envelope does not match the current snapshot")
            }
            Self::EvaluationEvidenceMismatch => {
                formatter.write_str("evaluation evidence does not match current evaluation")
            }
            Self::FailedEvaluation(id) => write!(formatter, "node {} evaluation failed", id.0),
            Self::RevisionExhausted => formatter.write_str("revision ID space is exhausted"),
            Self::Graph(error) => error.fmt(formatter),
            Self::Prismatic(error) => error.fmt(formatter),
            Self::Space(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CanonicalError {}

impl From<SketchError> for CanonicalError {
    fn from(error: SketchError) -> Self {
        Self::Sketch(error)
    }
}

impl From<PrismaticError> for CanonicalError {
    fn from(error: PrismaticError) -> Self {
        Self::Prismatic(error)
    }
}

impl From<SpaceError> for CanonicalError {
    fn from(error: SpaceError) -> Self {
        Self::Space(error)
    }
}

fn ensure_product_id(id: u64) -> Result<(), CanonicalError> {
    if id == 0 {
        Err(CanonicalError::ReservedProductId)
    } else {
        Ok(())
    }
}

fn ensure_name(name: &str) -> Result<(), CanonicalError> {
    if name.trim().is_empty() {
        Err(CanonicalError::EmptyProductName)
    } else {
        Ok(())
    }
}

fn validate_transform(transform: Transform) -> Result<(), CanonicalError> {
    Transform::from_matrix(transform.matrix).map(|_| ())
}

fn validate_persistent_dimension(dimension: &PersistentDimension) -> Result<(), CanonicalError> {
    ensure_product_id(dimension.id.0)?;
    ensure_name(&dimension.name)?;
    DimensionPresentation::new(
        dimension.presentation.unit,
        dimension.presentation.decimal_places,
    )?;
    if matches!(
        &dimension.target,
        PersistentDimensionTarget::FeatureParameter(FeatureParameterTarget {
            value_type: ParameterValueType::Angle | ParameterValueType::Scalar,
            ..
        }) | PersistentDimensionTarget::ExactFeatureParameter {
            value_type: ParameterValueType::Angle | ParameterValueType::Scalar,
            ..
        }
    ) || matches!(
        &dimension.target,
        PersistentDimensionTarget::ExactFeatureParameter {
            definition_id,
            producer_feature_id,
            semantic_role,
            source_element_id,
            ..
        } if definition_id.0 == 0
            || producer_feature_id.0 == 0
            || semantic_role.is_empty()
            || source_element_id.is_empty()
    ) {
        return Err(CanonicalError::InvalidPersistentDimensionTarget);
    }
    Ok(())
}

fn resolve_persistent_dimension(
    product: &ProductModel,
    dimension: &PersistentDimension,
) -> (DimensionReferenceHealth, Option<f64>) {
    match &dimension.target {
        PersistentDimensionTarget::FeatureParameter(target) => {
            let value = feature_parameter_value_bits(product, target).map(f64::from_bits);
            if value.is_some() {
                (DimensionReferenceHealth::Resolved, value)
            } else {
                (DimensionReferenceHealth::Lost, None)
            }
        }
        PersistentDimensionTarget::DerivedOutput(target) => {
            match resolve_derived_identity(&product.evaluator_nodes, target) {
                SlotResolution::Resolved => {
                    let value =
                        evaluate_graph(&product.evaluator_nodes, &EvaluationIdentity::default())
                            .ok()
                            .and_then(|report| {
                                report.outputs.get(target).map(|output| output.value)
                            });
                    if value.is_some() {
                        (DimensionReferenceHealth::Resolved, value)
                    } else {
                        (DimensionReferenceHealth::Lost, None)
                    }
                }
                SlotResolution::Ambiguous { segment_index } => {
                    (DimensionReferenceHealth::Ambiguous { segment_index }, None)
                }
                SlotResolution::Lost { .. } => (DimensionReferenceHealth::Lost, None),
            }
        }
        PersistentDimensionTarget::ExactFeatureParameter {
            definition_id,
            producer_feature_id,
            semantic_role,
            source_element_id,
            path,
            value_type,
        } => {
            let candidates = product
                .exact_reference_evidence
                .values()
                .filter(|reference| {
                    reference.document_id == product.document_id
                        && reference.definition_id == *definition_id
                        && reference.producer_feature_id == *producer_feature_id
                        && reference.semantic_role == *semantic_role
                        && reference.source_element_id == *source_element_id
                        && reference.has_valid_lineage()
                })
                .count();
            match candidates {
                0 => (DimensionReferenceHealth::Lost, None),
                1 => {
                    let value = feature_parameter_value_bits(
                        product,
                        &FeatureParameterTarget {
                            feature_id: *producer_feature_id,
                            path: path.clone(),
                            value_type: *value_type,
                        },
                    )
                    .map(f64::from_bits);
                    if value.is_some() {
                        (DimensionReferenceHealth::Resolved, value)
                    } else {
                        (DimensionReferenceHealth::Lost, None)
                    }
                }
                _ => (
                    DimensionReferenceHealth::Ambiguous { segment_index: 0 },
                    None,
                ),
            }
        }
    }
}

fn feature_supports_parameter_target(kind: &FeatureKind, target: &FeatureParameterTarget) -> bool {
    kind.parameter_descriptors().iter().any(|descriptor| {
        descriptor.path() == &target.path && descriptor.value_type() == target.value_type
    })
}

fn audit_feature_parameter_binding(
    product: &ProductModel,
    binding: &FeatureParameterBinding,
    identity: &EvaluationIdentity,
    report: &EvaluationReport,
) -> FeatureParameterFreshness {
    let Some(provenance) = product.feature_parameter_provenance.get(&binding.target) else {
        return FeatureParameterFreshness::Stale(FeatureParameterStaleReason::NeverComputed);
    };
    let reason = if provenance.identity.evaluator != identity.evaluator {
        Some(FeatureParameterStaleReason::EvaluatorChanged)
    } else if provenance.identity.schema != identity.schema {
        Some(FeatureParameterStaleReason::SchemaChanged)
    } else if provenance.identity.tolerance != identity.tolerance {
        Some(FeatureParameterStaleReason::ToleranceChanged)
    } else if provenance.identity.backend != identity.backend {
        Some(FeatureParameterStaleReason::BackendChanged)
    } else if let Some(output) = report.outputs.get(&binding.derived_from) {
        if provenance.input_digest != output.input_digest {
            Some(FeatureParameterStaleReason::InputChanged)
        } else if provenance.result_digest != output.result_digest {
            Some(FeatureParameterStaleReason::ResultChanged)
        } else if feature_parameter_value_bits(product, &binding.target)
            .is_none_or(|value_bits| value_bits != provenance.applied_value_bits)
        {
            Some(FeatureParameterStaleReason::AppliedValueChanged)
        } else {
            None
        }
    } else {
        Some(FeatureParameterStaleReason::EvaluationFailed)
    };
    reason.map_or(
        FeatureParameterFreshness::Current,
        FeatureParameterFreshness::Stale,
    )
}

fn feature_parameter_dimension(
    product: &ProductModel,
    target: &FeatureParameterTarget,
) -> Option<Dimension> {
    let feature = product.features.get(&target.feature_id)?;
    if !feature_supports_parameter_target(&feature.kind, target) {
        return None;
    }
    let value = feature_kind_parameter_value(&feature.kind, target.path.as_str())?;
    Dimension::new(value.to_string(), value).ok()
}

fn feature_parameter_value_bits(
    product: &ProductModel,
    target: &FeatureParameterTarget,
) -> Option<u64> {
    feature_parameter_dimension(product, target).map(|value| value.millimetres().to_bits())
}

fn feature_kind_parameter_value(kind: &FeatureKind, path: &str) -> Option<f64> {
    let parts = path.split('.').collect::<Vec<_>>();
    match kind {
        FeatureKind::Workplane(spec) => match (&spec.support, parts.as_slice()) {
            (WorkplaneSupport::Offset { distance, .. }, ["support", "offset", "distance"]) => {
                Some(distance.millimetres())
            }
            _ => None,
        },
        FeatureKind::Sketch(spec) => sketch_parameter_value(spec, &parts),
        FeatureKind::Profile { points_mm } => match parts.as_slice() {
            ["bounds", "width"] if is_axis_aligned_rectangle(points_mm) => {
                Some(points_mm[1][0] - points_mm[0][0])
            }
            ["bounds", "height"] if is_axis_aligned_rectangle(points_mm) => {
                Some(points_mm[3][1] - points_mm[0][1])
            }
            ["points", index, axis] => {
                point_coordinate(*points_mm.get(index.parse::<usize>().ok()?)?, axis)
            }
            _ => None,
        },
        FeatureKind::SegmentProfile { segments, .. } => segment_parameter_value(segments, &parts),
        FeatureKind::SplineProfile { control_points_mm } => match parts.as_slice() {
            ["control_points", index, axis] => {
                point_coordinate(*control_points_mm.get(index.parse::<usize>().ok()?)?, axis)
            }
            _ => None,
        },
        FeatureKind::Extrusion { height, .. } if path == "height" => Some(height.millimetres()),
        FeatureKind::Pad(spec) => {
            extent_parameter_value(&spec.extent, path.strip_prefix("extent.")?)
        }
        FeatureKind::SketchPocket(spec) => {
            extent_parameter_value(&spec.extent, path.strip_prefix("extent.")?)
        }
        FeatureKind::BottleProfileControl {
            body_radius,
            body_height,
            shoulder_rise,
            ..
        } => match path {
            "body_radius" => Some(body_radius.millimetres()),
            "body_height" => Some(body_height.millimetres()),
            "shoulder_rise" => Some(shoulder_rise.millimetres()),
            _ => None,
        },
        FeatureKind::Revolve {
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            ..
        } => match parts.as_slice() {
            ["axis_start", axis] => point_coordinate(*axis_start_mm, axis),
            ["axis_end", axis] => point_coordinate(*axis_end_mm, axis),
            ["angle"] => Some(*angle_degrees),
            _ => None,
        },
        FeatureKind::Shell { thickness, .. } | FeatureKind::TopologyShell { thickness, .. }
            if path == "thickness" =>
        {
            Some(thickness.millimetres())
        }
        FeatureKind::BottleEdgeFinish { amount, .. }
        | FeatureKind::TopologyEdgeFinish { amount, .. }
            if path == "amount" =>
        {
            Some(amount.millimetres())
        }
        FeatureKind::TopologyFaceOffset { distance, .. }
        | FeatureKind::PlanarOffset { distance, .. }
            if path == "distance" =>
        {
            Some(distance.millimetres())
        }
        FeatureKind::Pocket { depth, .. } if path == "depth" => Some(depth.millimetres()),
        FeatureKind::Loft { sections } => match parts.as_slice() {
            ["sections", index, "elevation"] => {
                Some(sections.get(index.parse::<usize>().ok()?)?.elevation_mm)
            }
            _ => None,
        },
        _ => None,
    }
}

fn sketch_parameter_value(spec: &SketchSpec, parts: &[&str]) -> Option<f64> {
    match parts {
        ["entities", id, point, axis] => {
            let id = id.parse::<u64>().ok()?;
            let entity = spec.entities.iter().find(|entity| entity.id().0 == id)?;
            match (entity, *point) {
                (
                    SketchEntity::Line { start_mm, .. } | SketchEntity::Arc { start_mm, .. },
                    "start",
                ) => point_coordinate(*start_mm, axis),
                (SketchEntity::Line { end_mm, .. } | SketchEntity::Arc { end_mm, .. }, "end") => {
                    point_coordinate(*end_mm, axis)
                }
                (
                    SketchEntity::Arc { center_mm, .. } | SketchEntity::Circle { center_mm, .. },
                    "center",
                ) => point_coordinate(*center_mm, axis),
                _ => None,
            }
        }
        ["entities", id, "radius"] => {
            let id = id.parse::<u64>().ok()?;
            spec.entities
                .iter()
                .find(|entity| entity.id().0 == id)
                .and_then(|entity| match entity {
                    SketchEntity::Circle { radius_mm, .. } => Some(*radius_mm),
                    _ => None,
                })
        }
        ["constraints", id, "value"] => {
            let id = id.parse::<u64>().ok()?;
            spec.constraints
                .iter()
                .find(|constraint| constraint.id.0 == id)
                .and_then(|constraint| match &constraint.kind {
                    SketchConstraintKind::Distance { value, .. }
                    | SketchConstraintKind::Radius { value, .. } => Some(value.millimetres()),
                    _ => None,
                })
        }
        ["constraints", id, "position", axis] => {
            let id = id.parse::<u64>().ok()?;
            spec.constraints
                .iter()
                .find(|constraint| constraint.id.0 == id)
                .and_then(|constraint| match &constraint.kind {
                    SketchConstraintKind::FixedPoint { position_mm, .. } => {
                        point_coordinate(*position_mm, axis)
                    }
                    _ => None,
                })
        }
        _ => None,
    }
}

fn segment_parameter_value(segments: &[ProfileSegment], parts: &[&str]) -> Option<f64> {
    let ["segments", index, point, axis] = parts else {
        return None;
    };
    let segment = segments.get(index.parse::<usize>().ok()?)?;
    match (segment, *point) {
        (
            ProfileSegment::Line { start_mm, .. } | ProfileSegment::CircularArc { start_mm, .. },
            "start",
        ) => point_coordinate(*start_mm, axis),
        (
            ProfileSegment::Line { end_mm, .. } | ProfileSegment::CircularArc { end_mm, .. },
            "end",
        ) => point_coordinate(*end_mm, axis),
        (ProfileSegment::CircularArc { center_mm, .. }, "center") => {
            point_coordinate(*center_mm, axis)
        }
        _ => None,
    }
}

fn extent_parameter_value(extent: &FeatureExtent, path: &str) -> Option<f64> {
    match (extent, path) {
        (FeatureExtent::Blind(distance) | FeatureExtent::Symmetric(distance), "distance") => {
            Some(distance.millimetres())
        }
        (FeatureExtent::Bidirectional { along, .. }, "along.distance") => {
            extent_end_parameter_value(along)
        }
        (FeatureExtent::Bidirectional { opposite, .. }, "opposite.distance") => {
            extent_end_parameter_value(opposite)
        }
        _ => None,
    }
}

fn extent_end_parameter_value(extent: &FeatureExtentEnd) -> Option<f64> {
    match extent {
        FeatureExtentEnd::Blind(distance) => Some(distance.millimetres()),
        FeatureExtentEnd::ThroughAll | FeatureExtentEnd::UpToFace(_) => None,
    }
}

fn point_coordinate(point: [f64; 2], axis: &str) -> Option<f64> {
    match axis {
        "x" => Some(point[0]),
        "y" => Some(point[1]),
        _ => None,
    }
}

fn recompute_feature_parameters(
    product: &mut ProductModel,
    identity: &EvaluationIdentity,
    affected_nodes: Option<&BTreeSet<NodeId>>,
    previous: Option<&EvaluationReport>,
) -> Result<EvaluationReport, CanonicalError> {
    let all_nodes;
    let affected = if let Some(affected) = affected_nodes {
        affected
    } else {
        all_nodes = product.evaluator_nodes.keys().cloned().collect();
        &all_nodes
    };
    let report = evaluate_affected(&product.evaluator_nodes, identity, previous, affected)
        .map_err(CanonicalError::Graph)?;
    let bindings = product
        .feature_parameter_bindings
        .values()
        .map(|binding| binding.as_ref().clone())
        .collect::<Vec<_>>();
    for binding in bindings {
        if affected_nodes
            .is_some_and(|affected| !affected.contains(&binding.derived_from.root_rule_node_id))
        {
            continue;
        }
        let output =
            report
                .outputs
                .get(&binding.derived_from)
                .ok_or(CanonicalError::FailedEvaluation(
                    binding.derived_from.root_rule_node_id,
                ))?;
        let dimension = Dimension::new(output.value.to_string(), output.value)?;
        set_feature_parameter(product, &binding.target, dimension)?;
        product.feature_parameter_provenance.insert(
            binding.target.clone(),
            Arc::new(FeatureParameterProvenance {
                identity: identity.clone(),
                input_digest: output.input_digest.clone(),
                result_digest: output.result_digest.clone(),
                applied_value_bits: output.value.to_bits(),
            }),
        );
    }
    Ok(report)
}

fn set_feature_parameter(
    product: &mut ProductModel,
    target: &FeatureParameterTarget,
    dimension: Dimension,
) -> Result<(), CanonicalError> {
    let feature = product
        .features
        .get(&target.feature_id)
        .ok_or(CanonicalError::FeatureNotFound(target.feature_id))?;
    if !feature_supports_parameter_target(&feature.kind, target) {
        return Err(CanonicalError::InvalidFeatureParameterBinding(
            target.clone(),
        ));
    }
    let mut kind = feature.kind.clone();
    if !set_feature_kind_parameter(&mut kind, target, &dimension)? {
        return Err(CanonicalError::InvalidFeatureParameterBinding(
            target.clone(),
        ));
    }
    validate_feature_kind(&kind)?;
    product.features.insert(
        target.feature_id,
        Arc::new(Feature {
            id: feature.id,
            definition_id: feature.definition_id,
            name: feature.name.clone(),
            kind,
        }),
    );
    Ok(())
}

fn set_feature_kind_parameter(
    kind: &mut FeatureKind,
    target: &FeatureParameterTarget,
    dimension: &Dimension,
) -> Result<bool, CanonicalError> {
    let path = target.path.as_str();
    let parts = path.split('.').collect::<Vec<_>>();
    let value = dimension.millimetres();
    let updated = match kind {
        FeatureKind::Workplane(spec) => match (&mut spec.support, parts.as_slice()) {
            (WorkplaneSupport::Offset { distance, .. }, ["support", "offset", "distance"]) => {
                *distance = dimension.clone();
                true
            }
            _ => false,
        },
        FeatureKind::Sketch(spec) => set_sketch_parameter(spec, &parts, dimension),
        FeatureKind::Profile { points_mm } => match parts.as_slice() {
            ["bounds", "width"] | ["bounds", "height"] => {
                *points_mm = resize_axis_aligned_rectangle(points_mm, target, value)?;
                true
            }
            ["points", index, axis] => points_mm
                .get_mut(index.parse::<usize>().ok().unwrap_or(usize::MAX))
                .is_some_and(|point| set_point_coordinate(point, axis, value)),
            _ => false,
        },
        FeatureKind::SegmentProfile { segments, .. } => {
            set_segment_parameter(segments, &parts, value)
        }
        FeatureKind::SplineProfile { control_points_mm } => match parts.as_slice() {
            ["control_points", index, axis] => control_points_mm
                .get_mut(index.parse::<usize>().ok().unwrap_or(usize::MAX))
                .is_some_and(|point| set_point_coordinate(point, axis, value)),
            _ => false,
        },
        FeatureKind::Extrusion { height, .. } if path == "height" => {
            *height = dimension.clone();
            true
        }
        FeatureKind::Pad(spec) => path
            .strip_prefix("extent.")
            .is_some_and(|path| set_extent_parameter(&mut spec.extent, path, dimension)),
        FeatureKind::SketchPocket(spec) => path
            .strip_prefix("extent.")
            .is_some_and(|path| set_extent_parameter(&mut spec.extent, path, dimension)),
        FeatureKind::BottleProfileControl {
            body_radius,
            body_height,
            shoulder_rise,
            ..
        } => match path {
            "body_radius" => {
                *body_radius = dimension.clone();
                true
            }
            "body_height" => {
                *body_height = dimension.clone();
                true
            }
            "shoulder_rise" => {
                *shoulder_rise = dimension.clone();
                true
            }
            _ => false,
        },
        FeatureKind::Revolve {
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            ..
        } => match parts.as_slice() {
            ["axis_start", axis] => set_point_coordinate(axis_start_mm, axis, value),
            ["axis_end", axis] => set_point_coordinate(axis_end_mm, axis, value),
            ["angle"] => {
                *angle_degrees = value;
                true
            }
            _ => false,
        },
        FeatureKind::Shell { thickness, .. } | FeatureKind::TopologyShell { thickness, .. }
            if path == "thickness" =>
        {
            *thickness = dimension.clone();
            true
        }
        FeatureKind::BottleEdgeFinish { amount, .. }
        | FeatureKind::TopologyEdgeFinish { amount, .. }
            if path == "amount" =>
        {
            *amount = dimension.clone();
            true
        }
        FeatureKind::TopologyFaceOffset { distance, .. }
        | FeatureKind::PlanarOffset { distance, .. }
            if path == "distance" =>
        {
            *distance = dimension.clone();
            true
        }
        FeatureKind::Pocket { depth, .. } if path == "depth" => {
            *depth = dimension.clone();
            true
        }
        FeatureKind::Loft { sections } => match parts.as_slice() {
            ["sections", index, "elevation"] => sections
                .get_mut(index.parse::<usize>().ok().unwrap_or(usize::MAX))
                .is_some_and(|section| {
                    section.elevation_mm = value;
                    true
                }),
            _ => false,
        },
        _ => false,
    };
    Ok(updated)
}

fn set_sketch_parameter(spec: &mut SketchSpec, parts: &[&str], dimension: &Dimension) -> bool {
    let value = dimension.millimetres();
    match parts {
        ["entities", id, point, axis] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            let Some(entity) = spec.entities.iter_mut().find(|entity| entity.id().0 == id) else {
                return false;
            };
            match (entity, *point) {
                (
                    SketchEntity::Line { start_mm, .. } | SketchEntity::Arc { start_mm, .. },
                    "start",
                ) => set_point_coordinate(start_mm, axis, value),
                (SketchEntity::Line { end_mm, .. } | SketchEntity::Arc { end_mm, .. }, "end") => {
                    set_point_coordinate(end_mm, axis, value)
                }
                (
                    SketchEntity::Arc { center_mm, .. } | SketchEntity::Circle { center_mm, .. },
                    "center",
                ) => set_point_coordinate(center_mm, axis, value),
                _ => false,
            }
        }
        ["entities", id, "radius"] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            spec.entities
                .iter_mut()
                .find(|entity| entity.id().0 == id)
                .is_some_and(|entity| match entity {
                    SketchEntity::Circle { radius_mm, .. } => {
                        *radius_mm = value;
                        true
                    }
                    _ => false,
                })
        }
        ["constraints", id, "value"] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            spec.constraints
                .iter_mut()
                .find(|constraint| constraint.id.0 == id)
                .is_some_and(|constraint| match &mut constraint.kind {
                    SketchConstraintKind::Distance { value, .. }
                    | SketchConstraintKind::Radius { value, .. } => {
                        *value = dimension.clone();
                        true
                    }
                    _ => false,
                })
        }
        ["constraints", id, "position", axis] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            spec.constraints
                .iter_mut()
                .find(|constraint| constraint.id.0 == id)
                .is_some_and(|constraint| match &mut constraint.kind {
                    SketchConstraintKind::FixedPoint { position_mm, .. } => {
                        set_point_coordinate(position_mm, axis, value)
                    }
                    _ => false,
                })
        }
        _ => false,
    }
}

fn set_segment_parameter(segments: &mut [ProfileSegment], parts: &[&str], value: f64) -> bool {
    let ["segments", index, point, axis] = parts else {
        return false;
    };
    let Ok(index) = index.parse::<usize>() else {
        return false;
    };
    let Some(segment) = segments.get_mut(index) else {
        return false;
    };
    match (segment, *point) {
        (
            ProfileSegment::Line { start_mm, .. } | ProfileSegment::CircularArc { start_mm, .. },
            "start",
        ) => set_point_coordinate(start_mm, axis, value),
        (
            ProfileSegment::Line { end_mm, .. } | ProfileSegment::CircularArc { end_mm, .. },
            "end",
        ) => set_point_coordinate(end_mm, axis, value),
        (ProfileSegment::CircularArc { center_mm, .. }, "center") => {
            set_point_coordinate(center_mm, axis, value)
        }
        _ => false,
    }
}

fn set_extent_parameter(extent: &mut FeatureExtent, path: &str, dimension: &Dimension) -> bool {
    match (extent, path) {
        (FeatureExtent::Blind(distance) | FeatureExtent::Symmetric(distance), "distance") => {
            *distance = dimension.clone();
            true
        }
        (FeatureExtent::Bidirectional { along, .. }, "along.distance") => {
            set_extent_end_parameter(along, dimension)
        }
        (FeatureExtent::Bidirectional { opposite, .. }, "opposite.distance") => {
            set_extent_end_parameter(opposite, dimension)
        }
        _ => false,
    }
}

fn set_extent_end_parameter(extent: &mut FeatureExtentEnd, dimension: &Dimension) -> bool {
    match extent {
        FeatureExtentEnd::Blind(distance) => {
            *distance = dimension.clone();
            true
        }
        FeatureExtentEnd::ThroughAll | FeatureExtentEnd::UpToFace(_) => false,
    }
}

fn set_point_coordinate(point: &mut [f64; 2], axis: &str, value: f64) -> bool {
    match axis {
        "x" => point[0] = value,
        "y" => point[1] = value,
        _ => return false,
    }
    true
}

const MAX_STABLE_SUBSHAPE_ROLE_BYTES: usize = 128;

fn validate_stable_subshape_role(role: &str) -> Result<(), CanonicalError> {
    if role.is_empty()
        || role.len() > MAX_STABLE_SUBSHAPE_ROLE_BYTES
        || !role.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-' | b'=' | b'(' | b')' | b',' | b':')
        })
    {
        return Err(CanonicalError::InvalidStableSubshapeRole);
    }
    Ok(())
}

fn roles_are_strictly_sorted<T: Ord>(roles: &[T]) -> bool {
    !roles.is_empty() && roles.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_topological_feature_references(
    references: &[TopologicalElementRef],
    expected_kind: TopologicalElementKind,
) -> Result<(), CanonicalError> {
    if references.len() > 64
        || !roles_are_strictly_sorted(references)
        || references
            .iter()
            .any(|reference| reference.kind != expected_kind || !reference.has_valid_lineage())
    {
        return Err(CanonicalError::InvalidTopologicalFeatureReference);
    }
    Ok(())
}

fn validate_topological_feature_context(
    document_id: DocumentId,
    definition_id: DefinitionId,
    kind: &FeatureKind,
) -> Result<(), CanonicalError> {
    let (target, references) = match kind {
        FeatureKind::TopologyShell {
            target,
            removed_faces,
            ..
        } => (*target, removed_faces.as_slice()),
        FeatureKind::TopologyEdgeFinish { target, edges, .. } => (*target, edges.as_slice()),
        _ => return Ok(()),
    };
    if references.iter().any(|reference| {
        reference.document_id != document_id
            || reference.definition_id != definition_id
            || reference.producer_feature_id != target
    }) {
        return Err(CanonicalError::InvalidTopologicalFeatureReference);
    }
    Ok(())
}

fn validate_topological_target(
    product: &ProductModel,
    definition: &Definition,
    feature_id: FeatureId,
    target_id: FeatureId,
    references: &[TopologicalElementRef],
) -> Result<(), CanonicalError> {
    let target = product
        .features
        .get(&target_id)
        .ok_or(CanonicalError::FeatureNotFound(target_id))?;
    let feature_position = definition
        .feature_ids
        .iter()
        .position(|candidate| *candidate == feature_id)
        .ok_or(CanonicalError::InvalidFeatureOwnership(feature_id))?;
    let target_position = definition
        .feature_ids
        .iter()
        .position(|candidate| *candidate == target_id);
    let sources_are_valid = references.iter().all(|reference| {
        product
            .features
            .get(&reference.source_feature_id)
            .is_some_and(|source| {
                source.definition_id == target.definition_id
                    && definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == source.id)
                        .is_some_and(|position| position <= target_position.unwrap_or(usize::MAX))
            })
    });
    if target_id == feature_id
        || target.definition_id != definition.id
        || !feature_kind_is_solid(&target.kind)
        || target_position.is_none_or(|position| position >= feature_position)
        || !sources_are_valid
    {
        return Err(CanonicalError::InvalidFeatureOwnership(feature_id));
    }
    Ok(())
}

fn feature_kind_is_solid(kind: &FeatureKind) -> bool {
    matches!(
        kind,
        FeatureKind::Extrusion { .. }
            | FeatureKind::Pad(_)
            | FeatureKind::SketchPocket(_)
            | FeatureKind::Revolve { .. }
            | FeatureKind::Shell { .. }
            | FeatureKind::BottleEdgeFinish { .. }
            | FeatureKind::TopologyShell { .. }
            | FeatureKind::TopologyEdgeFinish { .. }
            | FeatureKind::TopologyFaceOffset { .. }
            | FeatureKind::ThroughCut { .. }
            | FeatureKind::Pocket { .. }
            | FeatureKind::Boolean { .. }
            | FeatureKind::Sweep { .. }
            | FeatureKind::Loft { .. }
            | FeatureKind::ImportedExactBody(_)
            | FeatureKind::MeshBody(_)
    )
}

fn primary_solid_dependency(kind: &FeatureKind) -> Option<FeatureId> {
    match kind {
        FeatureKind::SketchPocket(spec) => Some(spec.target),
        FeatureKind::Shell { target, .. }
        | FeatureKind::BottleEdgeFinish { target, .. }
        | FeatureKind::TopologyShell { target, .. }
        | FeatureKind::TopologyEdgeFinish { target, .. }
        | FeatureKind::TopologyFaceOffset { target, .. }
        | FeatureKind::ThroughCut { target, .. }
        | FeatureKind::Pocket { target, .. }
        | FeatureKind::Boolean { target, .. } => Some(*target),
        _ => None,
    }
}

fn inferred_feature_body_ownership(
    _product: &ProductModel,
    definition: &Definition,
    kind: &FeatureKind,
) -> Result<FeatureBodyOwnership, CanonicalError> {
    let input_body_ids = kind
        .dependencies()
        .into_iter()
        .filter_map(|dependency| {
            definition
                .feature_body_ownership
                .get(&dependency)
                .and_then(FeatureBodyOwnership::output_body_id)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let output_body_id = if feature_kind_is_solid(kind) {
        primary_solid_dependency(kind)
            .and_then(|dependency| {
                definition
                    .feature_body_ownership
                    .get(&dependency)
                    .and_then(FeatureBodyOwnership::output_body_id)
            })
            .or(Some(definition.active_body_id))
    } else {
        None
    };
    FeatureBodyOwnership::new(input_body_ids, output_body_id)
}

fn feature_references_are_resolved(
    product: &ProductModel,
    id: FeatureId,
    visited: &mut BTreeSet<FeatureId>,
) -> bool {
    if !visited.insert(id) {
        return true;
    }
    let Some(feature) = product.features.get(&id) else {
        return false;
    };
    if matches!(
        &feature.kind,
        FeatureKind::Workplane(WorkplaneSpec {
            support: WorkplaneSupport::PlanarFace { health, .. },
            ..
        }) if *health != WorkplaneSupportHealth::Resolved
    ) {
        return false;
    }
    feature
        .kind
        .authoritative_dependencies()
        .into_iter()
        .all(|dependency| feature_references_are_resolved(product, dependency, visited))
}

fn validate_feature_body_ownership_change(
    product: &ProductModel,
    feature: &Feature,
    ownership: &FeatureBodyOwnership,
) -> Result<(), CanonicalError> {
    let definition = &product.definitions[&feature.definition_id];
    if ownership
        .input_body_ids
        .iter()
        .chain(ownership.output_body_id.iter())
        .any(|id| !definition.bodies.contains_key(id))
    {
        return Err(CanonicalError::BodyNotFound(
            definition.id,
            ownership
                .input_body_ids
                .iter()
                .chain(ownership.output_body_id.iter())
                .find(|id| !definition.bodies.contains_key(id))
                .cloned()
                .expect("a missing body was detected"),
        ));
    }
    if ownership
        .input_body_ids
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(CanonicalError::BodyInputsNotCanonical);
    }
    let inferred = inferred_feature_body_ownership(product, definition, &feature.kind)?;
    if ownership.input_body_ids != inferred.input_body_ids
        || feature_kind_is_solid(&feature.kind) != ownership.output_body_id.is_some()
    {
        return Err(CanonicalError::InvalidBodyOwnership(feature.id));
    }
    if !feature_references_are_resolved(product, feature.id, &mut BTreeSet::new()) {
        return Err(CanonicalError::UnresolvedBodyOwnershipReference(feature.id));
    }
    let mut candidate = definition.as_ref().clone();
    candidate
        .feature_body_ownership
        .insert(feature.id, ownership.clone());
    validate_body_dependency_graph(&candidate)?;
    Ok(())
}

fn ordered_body_feature_history(
    product: &ProductModel,
    definition_id: DefinitionId,
    body_id: BodyId,
    graph: &FeatureDependencyGraph,
) -> Result<Vec<FeatureId>, CanonicalError> {
    let definition = product
        .definitions
        .get(&definition_id)
        .ok_or(CanonicalError::DefinitionNotFound(definition_id))?;
    if !definition.bodies.contains_key(&body_id) {
        return Err(CanonicalError::BodyNotFound(definition_id, body_id));
    }
    let mut history = BTreeSet::new();
    let mut pending = graph
        .topological_order()
        .iter()
        .cloned()
        .filter(|id| {
            definition
                .feature_body_ownership
                .get(id)
                .and_then(FeatureBodyOwnership::output_body_id)
                == Some(body_id)
        })
        .collect::<Vec<_>>();
    while let Some(feature_id) = pending.pop() {
        if !history.insert(feature_id) {
            continue;
        }
        for dependency in graph
            .dependencies(feature_id)
            .ok_or(CanonicalError::FeatureNotFound(feature_id))?
        {
            let ownership = definition
                .feature_body_ownership
                .get(dependency)
                .ok_or(CanonicalError::FeatureNotFound(*dependency))?;
            if ownership
                .output_body_id()
                .is_none_or(|output| output == body_id)
            {
                pending.push(*dependency);
            }
        }
    }
    Ok(graph
        .topological_order()
        .iter()
        .cloned()
        .filter(|id| history.contains(id))
        .collect())
}

fn validate_body_feature_suppression(
    product: &ProductModel,
    definition_id: DefinitionId,
    body_id: BodyId,
    suppressed_feature_ids: &[FeatureId],
    graph: &FeatureDependencyGraph,
) -> Result<(), CanonicalError> {
    let ordered_history = ordered_body_feature_history(product, definition_id, body_id, graph)?;
    if suppressed_feature_ids.is_empty() {
        return Ok(());
    }
    let Some(boundary) = ordered_history
        .iter()
        .position(|id| *id == suppressed_feature_ids[0])
    else {
        return Err(CanonicalError::InvalidFeatureSuppression(
            definition_id,
            body_id,
        ));
    };
    if ordered_history[boundary..] != *suppressed_feature_ids {
        return Err(CanonicalError::InvalidFeatureSuppression(
            definition_id,
            body_id,
        ));
    }
    let suppressed = suppressed_feature_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if suppressed.len() != suppressed_feature_ids.len()
        || suppressed.iter().any(|id| {
            graph.dependents(*id).is_some_and(|dependents| {
                dependents
                    .iter()
                    .any(|dependent| !suppressed.contains(dependent))
            })
        })
    {
        return Err(CanonicalError::InvalidFeatureSuppression(
            definition_id,
            body_id,
        ));
    }
    Ok(())
}

fn validate_body_dependency_graph(definition: &Definition) -> Result<(), CanonicalError> {
    let mut edges = definition
        .bodies
        .keys()
        .cloned()
        .map(|id| (id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = definition
        .bodies
        .keys()
        .cloned()
        .map(|id| (id, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for ownership in definition.feature_body_ownership.values() {
        let Some(output) = ownership.output_body_id else {
            continue;
        };
        for input in &ownership.input_body_ids {
            if *input != output && edges.get_mut(input).is_some_and(|set| set.insert(output)) {
                *indegree
                    .get_mut(&output)
                    .expect("validated body output exists") += 1;
            }
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect::<BTreeSet<_>>();
    let mut visited = 0;
    while let Some(id) = ready.pop_first() {
        visited += 1;
        for dependent in &edges[&id] {
            let degree = indegree
                .get_mut(dependent)
                .expect("body dependency target exists");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(*dependent);
            }
        }
    }
    if visited != definition.bodies.len() {
        return Err(CanonicalError::BodyDependencyCycle(definition.id));
    }
    Ok(())
}

pub(crate) fn migrate_legacy_body_contract(
    product: &mut ProductModel,
) -> Result<(), CanonicalError> {
    let definition_ids = product.definitions.keys().cloned().collect::<Vec<_>>();
    for definition_id in definition_ids {
        let mut definition = product.definitions[&definition_id].as_ref().clone();
        definition.bodies = BTreeMap::from([(DEFAULT_BODY_ID, default_body())]);
        definition.active_body_id = DEFAULT_BODY_ID;
        definition.feature_body_ownership.clear();
        for feature_id in definition.feature_ids.clone() {
            let feature = product
                .features
                .get(&feature_id)
                .ok_or(CanonicalError::FeatureNotFound(feature_id))?;
            let ownership = inferred_feature_body_ownership(product, &definition, &feature.kind)?;
            definition
                .feature_body_ownership
                .insert(feature_id, ownership);
        }
        product
            .definitions
            .insert(definition_id, Arc::new(definition));
    }
    Ok(())
}

fn validate_feature_kind(kind: &FeatureKind) -> Result<(), CanonicalError> {
    match kind {
        FeatureKind::Workplane(spec) => {
            let mut canonical = spec.clone();
            if let WorkplaneSupport::PlanarFace {
                reference,
                health: _,
            } = &canonical.support
            {
                canonical.support = WorkplaneSupport::PlanarFace {
                    reference: reference.clone(),
                    health: WorkplaneSupportHealth::Resolved,
                };
            }
            canonical.validate_local().map_err(CanonicalError::from)
        }
        FeatureKind::Sketch(spec) => spec.solve().map(|_| ()).map_err(CanonicalError::from),
        FeatureKind::Profile { points_mm } => {
            if !is_valid_profile(points_mm) {
                return Err(CanonicalError::InvalidProfile);
            }
            Ok(())
        }
        FeatureKind::SegmentProfile { segments, closed } => {
            if !is_valid_segment_profile(segments, *closed) {
                return Err(CanonicalError::InvalidProfile);
            }
            Ok(())
        }
        FeatureKind::SplineProfile { control_points_mm } => {
            if !is_valid_profile(control_points_mm) || control_points_mm.len() < 4 {
                return Err(CanonicalError::InvalidSplineProfile);
            }
            Ok(())
        }
        FeatureKind::Extrusion { height, .. } => {
            Dimension::new(height.source_token.clone(), height.millimetres).map(|_| ())
        }
        FeatureKind::Pad(spec) => {
            spec.direction.validate().map_err(CanonicalError::from)?;
            spec.extent.validate().map_err(CanonicalError::from)
        }
        FeatureKind::SketchPocket(spec) => {
            spec.direction.validate().map_err(CanonicalError::from)?;
            spec.extent.validate().map_err(CanonicalError::from)?;
            if spec.support.expected_type != "planar_face"
                || spec.support.expected_cardinality != 1
                || !spec.support.has_valid_lineage()
            {
                return Err(CanonicalError::Sketch(
                    SketchError::InvalidPlanarFaceSupport,
                ));
            }
            Ok(())
        }
        FeatureKind::BottleProfileControl {
            body_radius,
            body_height,
            shoulder_rise,
            ..
        } => {
            for dimension in [body_radius, body_height, shoulder_rise] {
                Dimension::new(dimension.source_token.clone(), dimension.millimetres)
                    .map(|_| ())?;
                if dimension.millimetres <= 0.0 {
                    return Err(CanonicalError::DimensionOutsideEnvelope);
                }
            }
            Ok(())
        }
        FeatureKind::Pocket { depth, .. } => {
            Dimension::new(depth.source_token.clone(), depth.millimetres).map(|_| ())?;
            if depth.millimetres <= 0.0 {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            Ok(())
        }
        FeatureKind::Shell {
            removed_faces,
            thickness,
            ..
        } => {
            Dimension::new(thickness.source_token.clone(), thickness.millimetres).map(|_| ())?;
            if thickness.millimetres <= 0.0 {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            if !roles_are_strictly_sorted(removed_faces) {
                return Err(CanonicalError::SubshapeRolesNotCanonical);
            }
            Ok(())
        }
        FeatureKind::BottleEdgeFinish { edges, amount, .. } => {
            Dimension::new(amount.source_token.clone(), amount.millimetres).map(|_| ())?;
            if amount.millimetres <= 0.0 {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            if !roles_are_strictly_sorted(edges) {
                return Err(CanonicalError::SubshapeRolesNotCanonical);
            }
            Ok(())
        }
        FeatureKind::TopologyShell {
            removed_faces,
            thickness,
            ..
        } => {
            Dimension::new(thickness.source_token.clone(), thickness.millimetres).map(|_| ())?;
            if thickness.millimetres <= 0.0 {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            validate_topological_feature_references(removed_faces, TopologicalElementKind::Face)
        }
        FeatureKind::TopologyEdgeFinish { edges, amount, .. } => {
            Dimension::new(amount.source_token.clone(), amount.millimetres).map(|_| ())?;
            if amount.millimetres <= 0.0 {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            validate_topological_feature_references(edges, TopologicalElementKind::Edge)
        }
        FeatureKind::TopologyFaceOffset { face, distance, .. } => {
            Dimension::new(distance.source_token.clone(), distance.millimetres).map(|_| ())?;
            if distance.millimetres.abs() <= PROFILE_EPSILON_MM {
                return Err(CanonicalError::DimensionOutsideEnvelope);
            }
            validate_topological_feature_references(
                std::slice::from_ref(face),
                TopologicalElementKind::Face,
            )
        }
        FeatureKind::ImportedExactBody(spec) => validate_imported_exact_body(spec),
        FeatureKind::MeshBody(spec) => validate_mesh_body(spec),
        FeatureKind::Revolve {
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            ..
        } => {
            if axis_start_mm
                .iter()
                .chain(axis_end_mm)
                .any(|value| !value.is_finite() || value.abs() > MAX_CANONICAL_ABS_MM)
                || (axis_end_mm[0] - axis_start_mm[0]).hypot(axis_end_mm[1] - axis_start_mm[1])
                    <= PROFILE_EPSILON_MM
                || !angle_degrees.is_finite()
                || *angle_degrees <= 0.0
                || *angle_degrees > 360.0
            {
                return Err(CanonicalError::InvalidRevolve);
            }
            Ok(())
        }
        FeatureKind::PlanarOffset { distance, .. } => {
            Dimension::new(distance.source_token.clone(), distance.millimetres).map(|_| ())?;
            if distance.millimetres.abs() <= PROFILE_EPSILON_MM {
                return Err(CanonicalError::InvalidPlanarOffset);
            }
            Ok(())
        }
        FeatureKind::Sweep { profile, path } => {
            if profile == path {
                return Err(CanonicalError::InvalidSweep);
            }
            Ok(())
        }
        FeatureKind::Loft { sections } => {
            if !(2..=16).contains(&sections.len())
                || sections.windows(2).any(|pair| {
                    pair[0].elevation_mm >= pair[1].elevation_mm
                        || pair[0].profile == pair[1].profile
                })
                || sections.iter().any(|section| {
                    !section.elevation_mm.is_finite()
                        || section.elevation_mm.abs() > MAX_CANONICAL_ABS_MM
                })
                || sections
                    .iter()
                    .map(|section| section.profile)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != sections.len()
            {
                return Err(CanonicalError::InvalidLoft);
            }
            Ok(())
        }
        FeatureKind::ThroughCut { .. } | FeatureKind::Boolean { .. } => Ok(()),
    }
}

fn validate_imported_exact_body(spec: &ImportedExactBodySpec) -> Result<(), CanonicalError> {
    let bounds_valid = spec
        .bounds_mm
        .iter()
        .flatten()
        .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_CANONICAL_ABS_MM)
        && (0..3).all(|axis| spec.bounds_mm[0][axis] <= spec.bounds_mm[1][axis]);
    if spec.schema != IMPORTED_EXACT_BODY_SCHEMA_V1
        || spec.import_id.0 == 0
        || spec.source_byte_len == 0
        || spec.source_byte_len > 32 * 1024 * 1024
        || spec.source_sha256.iter().all(|byte| *byte == 0)
        || spec.result_fingerprint.is_empty()
        || spec.result_fingerprint.len() > 128
        || spec.solid_count == 0
        || spec.solid_count > 1_024
        || spec.topology_counts.is_some_and(|counts| {
            counts.contains(&0)
                || counts[4] != spec.solid_count
                || counts[..3]
                    .iter()
                    .map(|count| u64::from(*count))
                    .sum::<u64>()
                    > crate::topology::MAX_GENERATED_TOPOLOGICAL_REFERENCES
        })
        || !spec.volume_mm3.is_finite()
        || spec.volume_mm3 <= 0.0
        || !bounds_valid
        || spec.backend.is_empty()
        || spec.backend.len() > 1_024
        || spec.tolerance.is_empty()
        || spec.tolerance.len() > 1_024
    {
        return Err(CanonicalError::InvalidImportReceipt);
    }
    Ok(())
}

const MAX_MESH_VERTICES: usize = 100_000;
const MAX_MESH_TRIANGLES: usize = 200_000;
const MESH_AREA_EPSILON: f64 = 1.0e-18;
const MESH_VOLUME_EPSILON: f64 = 1.0e-12;

fn validate_mesh_body(spec: &MeshBodySpec) -> Result<(), CanonicalError> {
    if spec.schema != MESH_BODY_SCHEMA_V1
        || !(4..=MAX_MESH_VERTICES).contains(&spec.vertices_mm.len())
        || !(4..=MAX_MESH_TRIANGLES).contains(&spec.triangles.len())
        || spec
            .vertices_mm
            .iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_CANONICAL_ABS_MM)
    {
        return Err(CanonicalError::InvalidMeshBody);
    }
    match &spec.authority {
        MeshAuthority::Authored { provenance } if provenance.is_empty() => {
            return Err(CanonicalError::InvalidMeshBody);
        }
        MeshAuthority::ImportedStl { import_id }
        | MeshAuthority::ImportedSketchupScene { import_id }
            if import_id.0 == 0 =>
        {
            return Err(CanonicalError::InvalidMeshBody);
        }
        MeshAuthority::ExactConversion(conversion)
            if conversion.source_document_id.0 == 0
                || conversion.source_revision == 0
                || conversion.source_digest.is_empty()
                || conversion.source_definition_id.0 == 0
                || conversion.source_feature_id.0 == 0
                || conversion.source_result_fingerprint.is_empty()
                || conversion.source_evaluator.is_empty()
                || conversion.source_backend.is_empty()
                || conversion.source_tolerance.is_empty()
                || conversion.tessellation_tolerance.is_empty()
                || conversion.destination_definition_id.0 == 0
                || conversion.destination_feature_id.0 == 0
                || conversion.unsupported_semantics.is_empty()
                || !conversion
                    .unsupported_semantics
                    .windows(2)
                    .all(|pair| pair[0] < pair[1]) =>
        {
            return Err(CanonicalError::InvalidMeshBody);
        }
        _ => {}
    }

    let mut edges = BTreeMap::<(u32, u32), (u32, i32, Vec<usize>)>::new();
    let mut seen_triangles = BTreeSet::new();
    let volume_origin = spec.vertices_mm[0];
    let mut signed_volume_times_six = 0.0;
    let mut volume_compensation = 0.0;
    for (triangle_index, indices) in spec.triangles.iter().enumerate() {
        let [a, b, c] = *indices;
        if a == b
            || b == c
            || a == c
            || [a, b, c]
                .into_iter()
                .any(|index| index as usize >= spec.vertices_mm.len())
        {
            return Err(CanonicalError::InvalidMeshBody);
        }
        let mut canonical = [a, b, c];
        canonical.sort_unstable();
        if !seen_triangles.insert(canonical) {
            return Err(CanonicalError::InvalidMeshBody);
        }
        let first = spec.vertices_mm[a as usize];
        let second = spec.vertices_mm[b as usize];
        let third = spec.vertices_mm[c as usize];
        let first_edge = [
            second[0] - first[0],
            second[1] - first[1],
            second[2] - first[2],
        ];
        let second_edge = [
            third[0] - first[0],
            third[1] - first[1],
            third[2] - first[2],
        ];
        let cross = [
            first_edge[1] * second_edge[2] - first_edge[2] * second_edge[1],
            first_edge[2] * second_edge[0] - first_edge[0] * second_edge[2],
            first_edge[0] * second_edge[1] - first_edge[1] * second_edge[0],
        ];
        if cross.into_iter().map(|value| value * value).sum::<f64>() <= MESH_AREA_EPSILON {
            return Err(CanonicalError::InvalidMeshBody);
        }
        let shifted = [first, second, third].map(|point| {
            [
                point[0] - volume_origin[0],
                point[1] - volume_origin[1],
                point[2] - volume_origin[2],
            ]
        });
        let volume_term = shifted[0][0]
            * (shifted[1][1] * shifted[2][2] - shifted[1][2] * shifted[2][1])
            + shifted[0][1] * (shifted[1][2] * shifted[2][0] - shifted[1][0] * shifted[2][2])
            + shifted[0][2] * (shifted[1][0] * shifted[2][1] - shifted[1][1] * shifted[2][0]);
        let corrected = volume_term - volume_compensation;
        let next = signed_volume_times_six + corrected;
        volume_compensation = (next - signed_volume_times_six) - corrected;
        signed_volume_times_six = next;
        for (from, to) in [(a, b), (b, c), (c, a)] {
            let key = (from.min(to), from.max(to));
            let entry = edges.entry(key).or_default();
            entry.0 += 1;
            entry.1 += if from < to { 1 } else { -1 };
            entry.2.push(triangle_index);
        }
    }
    if edges
        .values()
        .any(|(uses, orientation, _)| *uses != 2 || *orientation != 0)
        || !mesh_vertex_fans_are_manifold(spec.vertices_mm.len(), &spec.triangles, &edges)
        || signed_volume_times_six <= MESH_VOLUME_EPSILON
    {
        return Err(CanonicalError::InvalidMeshBody);
    }
    Ok(())
}

fn mesh_vertex_fans_are_manifold(
    vertex_count: usize,
    triangles: &[[u32; 3]],
    edges: &BTreeMap<(u32, u32), (u32, i32, Vec<usize>)>,
) -> bool {
    let mut incident = vec![BTreeSet::new(); vertex_count];
    let mut adjacency = vec![BTreeMap::<usize, BTreeSet<usize>>::new(); vertex_count];
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        for vertex in triangle {
            incident[*vertex as usize].insert(triangle_index);
        }
    }
    for ((first, second), (_, _, uses)) in edges {
        if let [left, right] = uses.as_slice() {
            for vertex in [*first, *second] {
                adjacency[vertex as usize]
                    .entry(*left)
                    .or_default()
                    .insert(*right);
                adjacency[vertex as usize]
                    .entry(*right)
                    .or_default()
                    .insert(*left);
            }
        }
    }
    incident.iter().enumerate().all(|(vertex, faces)| {
        let Some(start) = faces.first().cloned() else {
            return false;
        };
        if faces.iter().any(|face| {
            adjacency[vertex]
                .get(face)
                .is_none_or(|neighbours| neighbours.len() != 2)
        }) {
            return false;
        }
        let mut visited = BTreeSet::new();
        let mut pending = vec![start];
        while let Some(face) = pending.pop() {
            if visited.insert(face)
                && let Some(neighbours) = adjacency[vertex].get(&face)
            {
                pending.extend(neighbours.iter().cloned());
            }
        }
        &visited == faces
    })
}

const MAX_PROFILE_POINTS: usize = 1_024;
const PROFILE_EPSILON_MM: f64 = 1.0e-9;

fn is_axis_aligned_rectangle(points_mm: &[[f64; 2]]) -> bool {
    points_mm.len() == 4
        && points_mm[0][1] == points_mm[1][1]
        && points_mm[1][0] == points_mm[2][0]
        && points_mm[2][1] == points_mm[3][1]
        && points_mm[3][0] == points_mm[0][0]
        && points_mm[1][0] > points_mm[0][0]
        && points_mm[3][1] > points_mm[0][1]
}

fn resize_axis_aligned_rectangle(
    points_mm: &[[f64; 2]],
    target: &FeatureParameterTarget,
    value_mm: f64,
) -> Result<Vec<[f64; 2]>, CanonicalError> {
    if !is_axis_aligned_rectangle(points_mm) {
        return Err(CanonicalError::InvalidFeatureParameterBinding(
            target.clone(),
        ));
    }
    if value_mm <= PROFILE_EPSILON_MM {
        return Err(CanonicalError::DimensionOutsideEnvelope);
    }
    let mut resized = points_mm.to_vec();
    match target.path.as_str() {
        "bounds.width" => {
            let right = points_mm[0][0] + value_mm;
            resized[1][0] = right;
            resized[2][0] = right;
        }
        "bounds.height" => {
            let top = points_mm[0][1] + value_mm;
            resized[2][1] = top;
            resized[3][1] = top;
        }
        _ => {
            return Err(CanonicalError::InvalidFeatureParameterBinding(
                target.clone(),
            ));
        }
    }
    validate_feature_kind(&FeatureKind::Profile {
        points_mm: resized.clone(),
    })?;
    Ok(resized)
}

fn is_valid_segment_profile(segments: &[ProfileSegment], closed: bool) -> bool {
    if segments.is_empty() || segments.len() > MAX_PROFILE_POINTS {
        return false;
    }
    let valid_point = |point: [f64; 2]| {
        point
            .into_iter()
            .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_CANONICAL_ABS_MM)
    };
    for segment in segments {
        let start = segment.start_mm();
        let end = segment.end_mm();
        if !valid_point(start)
            || !valid_point(end)
            || (start[0] - end[0]).hypot(start[1] - end[1]) <= PROFILE_EPSILON_MM
        {
            return false;
        }
        if let ProfileSegment::CircularArc { center_mm, .. } = segment {
            if !valid_point(*center_mm) {
                return false;
            }
            let start_radius = (start[0] - center_mm[0]).hypot(start[1] - center_mm[1]);
            let end_radius = (end[0] - center_mm[0]).hypot(end[1] - center_mm[1]);
            let radius_tolerance = PROFILE_EPSILON_MM * start_radius.max(end_radius).max(1.0);
            if start_radius <= PROFILE_EPSILON_MM
                || (start_radius - end_radius).abs() > radius_tolerance
            {
                return false;
            }
        }
    }
    if segments
        .windows(2)
        .any(|pair| pair[0].end_mm() != pair[1].start_mm())
    {
        return false;
    }
    if closed {
        if segments.len() < 2 {
            return false;
        }
        let end = segments.last().expect("non-empty profile").end_mm();
        let start = segments[0].start_mm();
        end == start
    } else {
        true
    }
}

fn is_valid_profile(points_mm: &[[f64; 2]]) -> bool {
    if !(3..=MAX_PROFILE_POINTS).contains(&points_mm.len())
        || points_mm
            .iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_CANONICAL_ABS_MM)
    {
        return false;
    }
    for (index, point) in points_mm.iter().enumerate() {
        if points_mm[index + 1..].iter().any(|candidate| {
            (point[0] - candidate[0]).abs() <= PROFILE_EPSILON_MM
                && (point[1] - candidate[1]).abs() <= PROFILE_EPSILON_MM
        }) {
            return false;
        }
    }
    let twice_area: f64 = points_mm
        .iter()
        .zip(points_mm.iter().cycle().skip(1))
        .take(points_mm.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum();
    if twice_area <= PROFILE_EPSILON_MM {
        return false;
    }
    for left_index in 0..points_mm.len() {
        let left_next = (left_index + 1) % points_mm.len();
        for right_index in (left_index + 1)..points_mm.len() {
            let right_next = (right_index + 1) % points_mm.len();
            if left_index == right_next || left_next == right_index {
                continue;
            }
            if segments_intersect(
                points_mm[left_index],
                points_mm[left_next],
                points_mm[right_index],
                points_mm[right_next],
            ) {
                return false;
            }
        }
    }
    true
}

fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }
    fn on_segment(a: [f64; 2], b: [f64; 2], point: [f64; 2]) -> bool {
        point[0] >= a[0].min(b[0]) - PROFILE_EPSILON_MM
            && point[0] <= a[0].max(b[0]) + PROFILE_EPSILON_MM
            && point[1] >= a[1].min(b[1]) - PROFILE_EPSILON_MM
            && point[1] <= a[1].max(b[1]) + PROFILE_EPSILON_MM
    }
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    if ((ab_c > PROFILE_EPSILON_MM && ab_d < -PROFILE_EPSILON_MM)
        || (ab_c < -PROFILE_EPSILON_MM && ab_d > PROFILE_EPSILON_MM))
        && ((cd_a > PROFILE_EPSILON_MM && cd_b < -PROFILE_EPSILON_MM)
            || (cd_a < -PROFILE_EPSILON_MM && cd_b > PROFILE_EPSILON_MM))
    {
        return true;
    }
    (ab_c.abs() <= PROFILE_EPSILON_MM && on_segment(a, b, c))
        || (ab_d.abs() <= PROFILE_EPSILON_MM && on_segment(a, b, d))
        || (cd_a.abs() <= PROFILE_EPSILON_MM && on_segment(c, d, a))
        || (cd_b.abs() <= PROFILE_EPSILON_MM && on_segment(c, d, b))
}

fn is_valid_revolve_profile(points_mm: &[[f64; 2]]) -> bool {
    is_valid_profile(points_mm)
        && points_mm.len() >= 4
        && points_mm
            .first()
            .is_some_and(|point| point[0].abs() <= PROFILE_EPSILON_MM)
        && points_mm
            .last()
            .is_some_and(|point| point[0].abs() <= PROFILE_EPSILON_MM)
        && points_mm[1..points_mm.len() - 1]
            .iter()
            .all(|point| point[0] > PROFILE_EPSILON_MM)
}

fn shell_thickness_is_conservative(points_mm: &[[f64; 2]], thickness_mm: f64) -> bool {
    is_valid_revolve_profile(points_mm)
        && thickness_mm.is_finite()
        && thickness_mm > 0.0
        && points_mm[1..points_mm.len() - 1]
            .iter()
            .map(|point| point[0])
            .reduce(f64::min)
            .is_some_and(|minimum_radius| thickness_mm < minimum_radius * 0.5)
        && points_mm
            .windows(2)
            .map(|edge| {
                let radius = edge[1][0] - edge[0][0];
                let height = edge[1][1] - edge[0][1];
                radius.hypot(height)
            })
            .filter(|length| *length > PROFILE_EPSILON_MM)
            .reduce(f64::min)
            .is_some_and(|minimum_edge| thickness_mm < minimum_edge * 0.5)
}

fn controlled_bottle_profile(
    points_mm: &[[f64; 2]],
    body_radius_mm: f64,
    body_height_mm: f64,
    shoulder_rise_mm: f64,
) -> Option<Vec<[f64; 2]>> {
    if points_mm.len() != 6
        || !is_valid_revolve_profile(points_mm)
        || !body_radius_mm.is_finite()
        || !body_height_mm.is_finite()
        || !shoulder_rise_mm.is_finite()
        || body_radius_mm <= points_mm[3][0]
        || body_height_mm <= 0.0
        || shoulder_rise_mm <= 0.0
    {
        return None;
    }
    let base_z = points_mm[0][1];
    let neck_height = points_mm[4][1] - points_mm[3][1];
    if neck_height <= 0.0 {
        return None;
    }
    let body_top_z = base_z + body_height_mm;
    let shoulder_top_z = body_top_z + shoulder_rise_mm;
    let top_z = shoulder_top_z + neck_height;
    let controlled = vec![
        [0.0, base_z],
        [body_radius_mm, base_z],
        [body_radius_mm, body_top_z],
        [points_mm[3][0], shoulder_top_z],
        [points_mm[4][0], top_z],
        [0.0, top_z],
    ];
    is_valid_revolve_profile(&controlled).then_some(controlled)
}

fn resolved_bottle_profile(product: &ProductModel, id: FeatureId) -> Option<Vec<[f64; 2]>> {
    let feature = product.features.get(&id)?;
    match &feature.kind {
        FeatureKind::Profile { points_mm } if is_valid_revolve_profile(points_mm) => {
            Some(points_mm.clone())
        }
        FeatureKind::BottleProfileControl {
            profile,
            body_radius,
            body_height,
            shoulder_rise,
        } => {
            let FeatureKind::Profile { points_mm } = &product.features.get(profile)?.kind else {
                return None;
            };
            controlled_bottle_profile(
                points_mm,
                body_radius.millimetres(),
                body_height.millimetres(),
                shoulder_rise.millimetres(),
            )
        }
        _ => None,
    }
}

fn clone_definition_and_repoint(
    product: &mut ProductModel,
    plan: &CloneDefinitionPlan,
) -> Result<(), CanonicalError> {
    let occurrence_id = plan.occurrence_id;
    let source_definition_id = plan.source_definition_id;
    let new_definition_id = plan.new_definition_id;
    let new_definition_name = &plan.new_definition_name;
    let feature_id_map = &plan.feature_id_map;
    ensure_product_id(new_definition_id.0)?;
    ensure_name(new_definition_name)?;
    if product.definitions.contains_key(&new_definition_id) {
        return Err(CanonicalError::DefinitionAlreadyExists(new_definition_id));
    }
    let occurrence = product
        .occurrences
        .get(&occurrence_id)
        .ok_or(CanonicalError::OccurrenceNotFound(occurrence_id))?
        .as_ref()
        .clone();
    if occurrence.definition_id != source_definition_id {
        return Err(CanonicalError::OccurrenceDefinitionMismatch);
    }
    let source = product
        .definitions
        .get(&source_definition_id)
        .ok_or(CanonicalError::DefinitionNotFound(source_definition_id))?
        .as_ref()
        .clone();
    if feature_id_map.len() != source.feature_ids.len()
        || feature_id_map
            .iter()
            .map(|(source_id, _)| *source_id)
            .ne(source.feature_ids.iter().cloned())
    {
        return Err(CanonicalError::InvalidFeatureMap);
    }
    let mut mapped_ids = BTreeSet::new();
    let mut mapping = BTreeMap::new();
    for (source_id, new_id) in feature_id_map {
        ensure_product_id(new_id.0)?;
        if !mapped_ids.insert(*new_id) || product.features.contains_key(new_id) {
            return Err(CanonicalError::InvalidFeatureMap);
        }
        mapping.insert(*source_id, *new_id);
    }

    let mut cloned_features = Vec::with_capacity(feature_id_map.len());
    for (source_id, new_id) in feature_id_map {
        let source_feature = product
            .features
            .get(source_id)
            .ok_or(CanonicalError::FeatureNotFound(*source_id))?;
        let kind = match &source_feature.kind {
            FeatureKind::Workplane(spec) => {
                let mut cloned = spec.clone();
                match &mut cloned.support {
                    WorkplaneSupport::Principal(_) => {}
                    WorkplaneSupport::Offset { base, .. } => {
                        *base = *mapping.get(base).ok_or(CanonicalError::InvalidFeatureMap)?;
                    }
                    WorkplaneSupport::PlanarFace { .. } => {
                        return Err(CanonicalError::InvalidFeatureMap);
                    }
                }
                FeatureKind::Workplane(cloned)
            }
            FeatureKind::Sketch(spec) => {
                let mut cloned = spec.clone();
                cloned.workplane = *mapping
                    .get(&spec.workplane)
                    .ok_or(CanonicalError::InvalidFeatureMap)?;
                FeatureKind::Sketch(cloned)
            }
            FeatureKind::Profile { points_mm } => FeatureKind::Profile {
                points_mm: points_mm.clone(),
            },
            FeatureKind::SegmentProfile { segments, closed } => FeatureKind::SegmentProfile {
                segments: segments.clone(),
                closed: *closed,
            },
            FeatureKind::SplineProfile { control_points_mm } => FeatureKind::SplineProfile {
                control_points_mm: control_points_mm.clone(),
            },
            FeatureKind::Extrusion { profile, height } => FeatureKind::Extrusion {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                height: height.clone(),
            },
            FeatureKind::Pad(spec) => {
                let mut cloned = spec.clone();
                cloned.sketch = *mapping
                    .get(&spec.sketch)
                    .ok_or(CanonicalError::InvalidFeatureMap)?;
                FeatureKind::Pad(cloned)
            }
            FeatureKind::SketchPocket(_) => return Err(CanonicalError::InvalidFeatureMap),
            FeatureKind::BottleProfileControl {
                profile,
                body_radius,
                body_height,
                shoulder_rise,
            } => FeatureKind::BottleProfileControl {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                body_radius: body_radius.clone(),
                body_height: body_height.clone(),
                shoulder_rise: shoulder_rise.clone(),
            },
            FeatureKind::Revolve {
                profile,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => FeatureKind::Revolve {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                axis_start_mm: *axis_start_mm,
                axis_end_mm: *axis_end_mm,
                angle_degrees: *angle_degrees,
            },
            FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
            } => FeatureKind::Shell {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                removed_faces: removed_faces.clone(),
                thickness: thickness.clone(),
            },
            FeatureKind::BottleEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => FeatureKind::BottleEdgeFinish {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                edges: edges.clone(),
                kind: *kind,
                amount: amount.clone(),
            },
            FeatureKind::TopologyShell { .. }
            | FeatureKind::TopologyEdgeFinish { .. }
            | FeatureKind::TopologyFaceOffset { .. } => {
                return Err(CanonicalError::InvalidFeatureMap);
            }
            FeatureKind::ThroughCut { target, profile } => FeatureKind::ThroughCut {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
            },
            FeatureKind::Pocket {
                target,
                profile,
                depth,
            } => FeatureKind::Pocket {
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                depth: depth.clone(),
            },
            FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => FeatureKind::Boolean {
                operation: *operation,
                target: *mapping
                    .get(target)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                tool: *mapping.get(tool).ok_or(CanonicalError::InvalidFeatureMap)?,
            },
            FeatureKind::PlanarOffset { profile, distance } => FeatureKind::PlanarOffset {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                distance: distance.clone(),
            },
            FeatureKind::Sweep { profile, path } => FeatureKind::Sweep {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                path: *mapping.get(path).ok_or(CanonicalError::InvalidFeatureMap)?,
            },
            FeatureKind::Loft { sections } => FeatureKind::Loft {
                sections: sections
                    .iter()
                    .map(|section| {
                        Ok(LoftSection {
                            profile: *mapping
                                .get(&section.profile)
                                .ok_or(CanonicalError::InvalidFeatureMap)?,
                            elevation_mm: section.elevation_mm,
                        })
                    })
                    .collect::<Result<Vec<_>, CanonicalError>>()?,
            },
            FeatureKind::ImportedExactBody(spec) => FeatureKind::ImportedExactBody(spec.clone()),
            FeatureKind::MeshBody(spec) => {
                let mut spec = spec.clone();
                if let MeshAuthority::ExactConversion(conversion) = &mut spec.authority {
                    conversion.destination_definition_id = new_definition_id;
                    conversion.destination_feature_id = *new_id;
                }
                FeatureKind::MeshBody(spec)
            }
        };
        cloned_features.push(Arc::new(Feature {
            id: *new_id,
            definition_id: new_definition_id,
            name: source_feature.name.clone(),
            kind,
        }));
    }

    let feature_body_ownership = source
        .feature_body_ownership
        .iter()
        .map(|(source_id, ownership)| {
            Ok((
                *mapping
                    .get(source_id)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                ownership.clone(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CanonicalError>>()?;
    let bodies = source
        .bodies
        .iter()
        .map(|(id, body)| {
            Ok((
                *id,
                Body {
                    consumed_by: body
                        .consumed_by
                        .map(|feature_id| {
                            mapping
                                .get(&feature_id)
                                .cloned()
                                .ok_or(CanonicalError::InvalidFeatureMap)
                        })
                        .transpose()?,
                    ..body.clone()
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CanonicalError>>()?;
    product.definitions.insert(
        new_definition_id,
        Arc::new(Definition {
            id: new_definition_id,
            name: new_definition_name.to_owned(),
            feature_ids: feature_id_map.iter().map(|(_, new_id)| *new_id).collect(),
            bodies,
            active_body_id: source.active_body_id,
            feature_body_ownership,
            local_occurrence_ids: source.local_occurrence_ids.clone(),
            local_group_ids: source.local_group_ids.clone(),
        }),
    );
    for local_id in &source.local_group_ids {
        let old_key = LocalGroupKey {
            definition_id: source_definition_id,
            local_id: *local_id,
        };
        let local = product.local_groups[&old_key].as_ref();
        let key = LocalGroupKey {
            definition_id: new_definition_id,
            local_id: *local_id,
        };
        product.local_groups.insert(
            key,
            Arc::new(LocalGroup {
                key,
                name: local.name.clone(),
                transform: local.transform,
                parent: local.parent,
            }),
        );
    }
    for local_id in &source.local_occurrence_ids {
        let old_key = LocalOccurrenceKey {
            definition_id: source_definition_id,
            local_id: *local_id,
        };
        let local = product.local_occurrences[&old_key].as_ref();
        let key = LocalOccurrenceKey {
            definition_id: new_definition_id,
            local_id: *local_id,
        };
        product.local_occurrences.insert(
            key,
            Arc::new(LocalOccurrence {
                key,
                definition_id: local.definition_id,
                name: local.name.clone(),
                transform: local.transform,
                parent: local.parent,
                tag: local.tag,
                visible: local.visible,
            }),
        );
    }
    for feature in cloned_features {
        product.features.insert(feature.id, feature);
    }
    for ((_, body_id), suppressed) in product
        .body_feature_suppression
        .clone()
        .into_iter()
        .filter(|((definition_id, _), _)| *definition_id == source_definition_id)
    {
        let mapped = suppressed
            .into_iter()
            .map(|feature_id| {
                mapping
                    .get(&feature_id)
                    .cloned()
                    .ok_or(CanonicalError::InvalidFeatureMap)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        product
            .body_feature_suppression
            .insert((new_definition_id, body_id), mapped);
    }
    let cloned_bindings = product
        .feature_parameter_bindings
        .values()
        .filter_map(|binding| {
            mapping
                .get(&binding.target.feature_id)
                .map(|new_feature_id| {
                    let target = FeatureParameterTarget {
                        feature_id: *new_feature_id,
                        path: binding.target.path.clone(),
                        value_type: binding.target.value_type,
                    };
                    (
                        target.clone(),
                        Arc::new(FeatureParameterBinding {
                            target,
                            derived_from: binding.derived_from.clone(),
                        }),
                    )
                })
        })
        .collect::<Vec<_>>();
    product.feature_parameter_bindings.extend(cloned_bindings);
    product.occurrences.insert(
        occurrence_id,
        Arc::new(Occurrence {
            definition_id: new_definition_id,
            ..occurrence
        }),
    );
    Ok(())
}

fn apply_solid_tool(
    product: &mut ProductModel,
    plan: &SolidToolPlan,
) -> Result<(), CanonicalError> {
    ensure_name(&plan.result_definition_name)?;
    ensure_name(&plan.result_feature_name)?;
    ensure_product_id(plan.result_definition_id.0)?;
    if plan.target_occurrence_id == plan.tool_occurrence_id
        || (plan.operation == BooleanOperation::Split && !plan.keep_tool)
        || product.definitions.contains_key(&plan.result_definition_id)
    {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    let mut output_ids = BTreeSet::new();
    for id in plan.result_feature_ids {
        ensure_product_id(id.0)?;
        if !output_ids.insert(id) || product.features.contains_key(&id) {
            return Err(CanonicalError::InvalidSolidToolPlan);
        }
    }
    if !plan.keep_tool
        && product
            .collections
            .values()
            .any(|collection| collection.occurrence_ids.contains(&plan.tool_occurrence_id))
    {
        return Err(CanonicalError::OccurrenceInCollection(
            plan.tool_occurrence_id,
        ));
    }

    let target_occurrence = product
        .occurrences
        .get(&plan.target_occurrence_id)
        .ok_or(CanonicalError::OccurrenceNotFound(
            plan.target_occurrence_id,
        ))?
        .as_ref()
        .clone();
    let tool_occurrence = product
        .occurrences
        .get(&plan.tool_occurrence_id)
        .ok_or(CanonicalError::OccurrenceNotFound(plan.tool_occurrence_id))?
        .as_ref()
        .clone();
    let target_feature = product
        .features
        .get(&plan.target_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(plan.target_feature_id))?;
    let tool_feature = product
        .features
        .get(&plan.tool_feature_id)
        .ok_or(CanonicalError::FeatureNotFound(plan.tool_feature_id))?;
    if target_feature.definition_id != target_occurrence.definition_id
        || tool_feature.definition_id != tool_occurrence.definition_id
    {
        return Err(CanonicalError::OccurrenceDefinitionMismatch);
    }
    let (target_profile_id, target_height) = match &target_feature.kind {
        FeatureKind::Extrusion { profile, height } => (*profile, height.clone()),
        _ => return Err(CanonicalError::InvalidSolidToolPlan),
    };
    let (tool_profile_id, tool_height) = match &tool_feature.kind {
        FeatureKind::Extrusion { profile, height } => (*profile, height.clone()),
        _ => return Err(CanonicalError::InvalidSolidToolPlan),
    };
    if target_height.millimetres().to_bits() != tool_height.millimetres().to_bits() {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }
    let target_profile = product
        .features
        .get(&target_profile_id)
        .ok_or(CanonicalError::FeatureNotFound(target_profile_id))?;
    let tool_profile = product
        .features
        .get(&tool_profile_id)
        .ok_or(CanonicalError::FeatureNotFound(tool_profile_id))?;
    let FeatureKind::Profile {
        points_mm: target_points,
    } = &target_profile.kind
    else {
        return Err(CanonicalError::InvalidSolidToolPlan);
    };
    if target_profile.definition_id != target_occurrence.definition_id
        || tool_profile.definition_id != tool_occurrence.definition_id
    {
        return Err(CanonicalError::OccurrenceDefinitionMismatch);
    }

    let snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    let target_transform = snapshot
        .world_transform_for_occurrence(plan.target_occurrence_id)
        .ok_or(CanonicalError::UnsupportedSolidToolTransform)?;
    let tool_transform = snapshot
        .world_transform_for_occurrence(plan.tool_occurrence_id)
        .ok_or(CanonicalError::UnsupportedSolidToolTransform)?;
    let translation_only = |transform: Transform| {
        let matrix = transform.matrix();
        matrix[0] == 1.0
            && matrix[1] == 0.0
            && matrix[2] == 0.0
            && matrix[4] == 0.0
            && matrix[5] == 1.0
            && matrix[6] == 0.0
            && matrix[8] == 0.0
            && matrix[9] == 0.0
            && matrix[10] == 1.0
    };
    if !translation_only(target_transform)
        || !translation_only(tool_transform)
        || target_transform.matrix()[11].to_bits() != tool_transform.matrix()[11].to_bits()
    {
        return Err(CanonicalError::UnsupportedSolidToolTransform);
    }
    let delta_x = tool_transform.matrix()[3] - target_transform.matrix()[3];
    let delta_y = tool_transform.matrix()[7] - target_transform.matrix()[7];
    let shifted_tool_profile = translated_solid_tool_profile(&tool_profile.kind, delta_x, delta_y)?;
    validate_feature_kind(&shifted_tool_profile)?;
    if !solid_tool_profiles_supported(plan.operation, target_points, &shifted_tool_profile) {
        return Err(CanonicalError::InvalidSolidToolPlan);
    }

    let [
        target_profile_output,
        target_body_output,
        tool_profile_output,
        tool_body_output,
        result_output,
    ] = plan.result_feature_ids;
    let features = [
        Feature {
            id: target_profile_output,
            definition_id: plan.result_definition_id,
            name: target_profile.name.clone(),
            kind: FeatureKind::Profile {
                points_mm: target_points.clone(),
            },
        },
        Feature {
            id: target_body_output,
            definition_id: plan.result_definition_id,
            name: target_feature.name.clone(),
            kind: FeatureKind::Extrusion {
                profile: target_profile_output,
                height: target_height,
            },
        },
        Feature {
            id: tool_profile_output,
            definition_id: plan.result_definition_id,
            name: tool_profile.name.clone(),
            kind: shifted_tool_profile,
        },
        Feature {
            id: tool_body_output,
            definition_id: plan.result_definition_id,
            name: tool_feature.name.clone(),
            kind: FeatureKind::Extrusion {
                profile: tool_profile_output,
                height: tool_height,
            },
        },
        Feature {
            id: result_output,
            definition_id: plan.result_definition_id,
            name: plan.result_feature_name.clone(),
            kind: FeatureKind::Boolean {
                operation: plan.operation,
                target: target_body_output,
                tool: tool_body_output,
            },
        },
    ];
    product.definitions.insert(
        plan.result_definition_id,
        Arc::new(Definition {
            id: plan.result_definition_id,
            name: plan.result_definition_name.clone(),
            feature_ids: plan.result_feature_ids.to_vec(),
            bodies: BTreeMap::from([(DEFAULT_BODY_ID, default_body())]),
            active_body_id: DEFAULT_BODY_ID,
            feature_body_ownership: BTreeMap::new(),
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }),
    );
    for feature in features {
        product.features.insert(feature.id, Arc::new(feature));
    }
    let mut result_definition = product.definitions[&plan.result_definition_id]
        .as_ref()
        .clone();
    for feature_id in result_definition.feature_ids.clone() {
        let feature = &product.features[&feature_id];
        let ownership =
            inferred_feature_body_ownership(product, &result_definition, &feature.kind)?;
        result_definition
            .feature_body_ownership
            .insert(feature_id, ownership);
    }
    product
        .definitions
        .insert(plan.result_definition_id, Arc::new(result_definition));
    let binding_mappings = [
        (target_profile_id, target_profile_output),
        (plan.target_feature_id, target_body_output),
        (tool_profile_id, tool_profile_output),
        (plan.tool_feature_id, tool_body_output),
    ];
    let cloned_bindings = binding_mappings
        .into_iter()
        .flat_map(|(source_id, output_id)| {
            product
                .feature_parameter_bindings
                .values()
                .filter(move |binding| binding.target.feature_id == source_id)
                .map(move |binding| {
                    let target = FeatureParameterTarget {
                        feature_id: output_id,
                        path: binding.target.path.clone(),
                        value_type: binding.target.value_type,
                    };
                    (
                        target.clone(),
                        Arc::new(FeatureParameterBinding {
                            target,
                            derived_from: binding.derived_from.clone(),
                        }),
                    )
                })
        })
        .collect::<Vec<_>>();
    product.feature_parameter_bindings.extend(cloned_bindings);
    product.occurrences.insert(
        plan.target_occurrence_id,
        Arc::new(Occurrence {
            definition_id: plan.result_definition_id,
            ..target_occurrence
        }),
    );
    if !plan.keep_tool {
        product.occurrences.remove(&plan.tool_occurrence_id);
    }
    Ok(())
}

fn translated_solid_tool_profile(
    profile: &FeatureKind,
    delta_x: f64,
    delta_y: f64,
) -> Result<FeatureKind, CanonicalError> {
    match profile {
        FeatureKind::Profile { points_mm } => Ok(FeatureKind::Profile {
            points_mm: points_mm
                .iter()
                .map(|point| [point[0] + delta_x, point[1] + delta_y])
                .collect(),
        }),
        FeatureKind::SegmentProfile {
            segments,
            closed: true,
        } if circle_segment_profile_bounds(segments).is_some()
            || line_arc_d_profile_bounds(segments).is_some()
            || is_line_arc_capsule_profile(segments, true)
            || strict_convex_line_arc_profile_bounds(segments, true).is_some()
            || line_segment_polygon_bounds(segments).is_some() =>
        {
            Ok(FeatureKind::SegmentProfile {
                segments: segments
                    .iter()
                    .map(|segment| match segment {
                        ProfileSegment::CircularArc {
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        } => ProfileSegment::CircularArc {
                            start_mm: [start_mm[0] + delta_x, start_mm[1] + delta_y],
                            end_mm: [end_mm[0] + delta_x, end_mm[1] + delta_y],
                            center_mm: [center_mm[0] + delta_x, center_mm[1] + delta_y],
                            clockwise: *clockwise,
                        },
                        ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
                            start_mm: [start_mm[0] + delta_x, start_mm[1] + delta_y],
                            end_mm: [end_mm[0] + delta_x, end_mm[1] + delta_y],
                        },
                    })
                    .collect(),
                closed: true,
            })
        }
        _ => Err(CanonicalError::InvalidSolidToolPlan),
    }
}

fn circle_segment_profile_bounds(segments: &[ProfileSegment]) -> Option<[f64; 4]> {
    let [
        ProfileSegment::CircularArc {
            start_mm: first_start,
            end_mm: first_end,
            center_mm: first_center,
            clockwise: first_clockwise,
        },
        ProfileSegment::CircularArc {
            start_mm: second_start,
            end_mm: second_end,
            center_mm: second_center,
            clockwise: second_clockwise,
        },
    ] = segments
    else {
        return None;
    };
    if first_start != second_end
        || first_end != second_start
        || first_center != second_center
        || first_clockwise != second_clockwise
    {
        return None;
    }
    let first_vector = [
        first_start[0] - first_center[0],
        first_start[1] - first_center[1],
    ];
    let end_vector = [
        first_end[0] - first_center[0],
        first_end[1] - first_center[1],
    ];
    if first_vector[0] != -end_vector[0] || first_vector[1] != -end_vector[1] {
        return None;
    }
    let radius = first_vector[0].hypot(first_vector[1]);
    (radius.is_finite() && radius > 0.0).then_some([
        first_center[0] - radius,
        first_center[1] - radius,
        first_center[0] + radius,
        first_center[1] + radius,
    ])
}

fn line_arc_d_profile_bounds(segments: &[ProfileSegment]) -> Option<[f64; 4]> {
    let (line_start, line_end, arc_start, arc_end, center) = match segments {
        [
            ProfileSegment::Line { start_mm, end_mm },
            ProfileSegment::CircularArc {
                start_mm: arc_start,
                end_mm: arc_end,
                center_mm,
                ..
            },
        ] => (*start_mm, *end_mm, *arc_start, *arc_end, *center_mm),
        [
            ProfileSegment::CircularArc {
                start_mm: arc_start,
                end_mm: arc_end,
                center_mm,
                ..
            },
            ProfileSegment::Line { start_mm, end_mm },
        ] => (*start_mm, *end_mm, *arc_start, *arc_end, *center_mm),
        _ => return None,
    };
    if line_end != arc_start || arc_end != line_start {
        return None;
    }
    let start_radius = (arc_start[0] - center[0]).hypot(arc_start[1] - center[1]);
    let end_radius = (arc_end[0] - center[0]).hypot(arc_end[1] - center[1]);
    if !start_radius.is_finite()
        || start_radius <= PROFILE_EPSILON_MM
        || (start_radius - end_radius).abs() > PROFILE_EPSILON_MM
    {
        return None;
    }
    Some([
        center[0] - start_radius,
        center[1] - start_radius,
        center[0] + start_radius,
        center[1] + start_radius,
    ])
}

fn line_segment_polygon_bounds(segments: &[ProfileSegment]) -> Option<[f64; 4]> {
    let points = segments
        .iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, .. } => Some(*start_mm),
            ProfileSegment::CircularArc { .. } => None,
        })
        .collect::<Option<Vec<_>>>()?;
    if segments.len() < 3
        || segments
            .windows(2)
            .any(|pair| pair[0].end_mm() != pair[1].start_mm())
        || segments.last()?.end_mm() != segments.first()?.start_mm()
        || !is_valid_profile(&points)
    {
        return None;
    }
    let min_x = points.iter().map(|point| point[0]).reduce(f64::min)?;
    let min_y = points.iter().map(|point| point[1]).reduce(f64::min)?;
    let max_x = points.iter().map(|point| point[0]).reduce(f64::max)?;
    let max_y = points.iter().map(|point| point[1]).reduce(f64::max)?;
    Some([min_x, min_y, max_x, max_y])
}

fn line_arc_d_profile_contains_rectangle(
    segments: &[ProfileSegment],
    width: f64,
    depth: f64,
) -> bool {
    if line_arc_d_profile_bounds(segments).is_none() {
        return false;
    }
    let mut polygon = Vec::new();
    for segment in segments {
        match segment {
            ProfileSegment::Line { start_mm, .. } => polygon.push(*start_mm),
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
                let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
                let mut sweep = end_angle - start_angle;
                if *clockwise {
                    while sweep >= 0.0 {
                        sweep -= std::f64::consts::TAU;
                    }
                } else {
                    while sweep <= 0.0 {
                        sweep += std::f64::consts::TAU;
                    }
                }
                if sweep.abs() >= std::f64::consts::TAU - 1.0e-12 {
                    return false;
                }
                let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                let steps = (sweep.abs() / std::f64::consts::TAU * 256.0)
                    .ceil()
                    .max(1.0) as usize;
                for step in 0..steps {
                    let angle = start_angle + sweep * step as f64 / steps as f64;
                    polygon.push([
                        center_mm[0] + radius * angle.cos(),
                        center_mm[1] + radius * angle.sin(),
                    ]);
                }
            }
        }
    }
    [[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]]
        .into_iter()
        .all(|point| point_strictly_in_polygon(point, &polygon))
}

fn line_segment_polygon_contains_rectangle(
    segments: &[ProfileSegment],
    width: f64,
    depth: f64,
) -> bool {
    let Some(points) = segments
        .iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, .. } => Some(*start_mm),
            ProfileSegment::CircularArc { .. } => None,
        })
        .collect::<Option<Vec<_>>>()
        .filter(|points| points.len() >= 3)
    else {
        return false;
    };
    [[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]]
        .into_iter()
        .all(|point| point_in_polygon_or_boundary(point, &points))
}

fn point_strictly_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let near_boundary = polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .any(|(start, end)| {
            let edge_x = end[0] - start[0];
            let edge_y = end[1] - start[1];
            let cross = edge_x * (point[1] - start[1]) - edge_y * (point[0] - start[0]);
            cross.abs() <= 1.0e-6 * edge_x.hypot(edge_y)
                && point[0] >= start[0].min(end[0]) - 1.0e-6
                && point[0] <= start[0].max(end[0]) + 1.0e-6
                && point[1] >= start[1].min(end[1]) - 1.0e-6
                && point[1] <= start[1].max(end[1]) + 1.0e-6
        });
    !near_boundary && point_in_polygon_or_boundary(point, polygon)
}

fn point_in_polygon_or_boundary(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    for (start, end) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        let cross = (end[0] - start[0]) * (point[1] - start[1])
            - (end[1] - start[1]) * (point[0] - start[0]);
        if cross.abs() <= 1.0e-9
            && point[0] >= start[0].min(end[0]) - 1.0e-9
            && point[0] <= start[0].max(end[0]) + 1.0e-9
            && point[1] >= start[1].min(end[1]) - 1.0e-9
            && point[1] <= start[1].max(end[1]) + 1.0e-9
        {
            return true;
        }
        if (start[1] > point[1]) != (end[1] > point[1])
            && point[0]
                < (end[0] - start[0]) * (point[1] - start[1]) / (end[1] - start[1]) + start[0]
        {
            inside = !inside;
        }
    }
    inside
}

fn line_segment_profile_bounds(segments: &[ProfileSegment]) -> Option<[f64; 4]> {
    if segments.len() != 4 {
        return None;
    }
    let mut points = Vec::with_capacity(4);
    for segment in segments {
        let ProfileSegment::Line { start_mm, end_mm } = segment else {
            return None;
        };
        let delta = [end_mm[0] - start_mm[0], end_mm[1] - start_mm[1]];
        if delta.iter().any(|value| !value.is_finite())
            || (delta[0].abs() > 2.0e-6 && delta[1].abs() > 2.0e-6)
        {
            return None;
        }
        points.push(*start_mm);
    }
    let min_x = points.iter().map(|point| point[0]).reduce(f64::min)?;
    let min_y = points.iter().map(|point| point[1]).reduce(f64::min)?;
    let max_x = points.iter().map(|point| point[0]).reduce(f64::max)?;
    let max_y = points.iter().map(|point| point[1]).reduce(f64::max)?;
    let corners = [
        [min_x, min_y],
        [min_x, max_y],
        [max_x, min_y],
        [max_x, max_y],
    ];
    (min_x < max_x
        && min_y < max_y
        && corners.iter().all(|corner| {
            points.iter().any(|point| {
                (point[0] - corner[0]).abs() <= 2.0e-6 && (point[1] - corner[1]).abs() <= 2.0e-6
            })
        }))
    .then_some([min_x, min_y, max_x, max_y])
}

fn solid_tool_profiles_supported(
    operation: BooleanOperation,
    target: &[[f64; 2]],
    tool: &FeatureKind,
) -> bool {
    if !is_axis_aligned_rectangle(target) || target[0] != [0.0, 0.0] {
        return false;
    }
    let tool_is_line_arc_d = matches!(
        tool,
        FeatureKind::SegmentProfile { segments, closed: true }
            if line_arc_d_profile_bounds(segments).is_some()
    );
    if tool_is_line_arc_d
        && !matches!(
            operation,
            BooleanOperation::Cut
                | BooleanOperation::Union
                | BooleanOperation::Intersect
                | BooleanOperation::Split
        )
    {
        return false;
    }
    let tool_is_capsule = matches!(
        tool,
        FeatureKind::SegmentProfile { segments, closed: true }
            if is_line_arc_capsule_profile(segments, true)
    );
    if tool_is_capsule
        && !matches!(
            operation,
            BooleanOperation::Cut | BooleanOperation::Union | BooleanOperation::Intersect
        )
    {
        return false;
    }
    let tool_bounds = match tool {
        FeatureKind::Profile { points_mm } if is_axis_aligned_rectangle(points_mm) => [
            points_mm[0][0],
            points_mm[0][1],
            points_mm[2][0],
            points_mm[2][1],
        ],
        FeatureKind::SegmentProfile {
            segments,
            closed: true,
        } => match circle_segment_profile_bounds(segments)
            .or_else(|| line_segment_profile_bounds(segments))
            .or_else(|| {
                matches!(
                    operation,
                    BooleanOperation::Cut
                        | BooleanOperation::Union
                        | BooleanOperation::Intersect
                        | BooleanOperation::Split
                )
                .then(|| line_arc_d_profile_bounds(segments))
                .flatten()
            })
            .or_else(|| {
                matches!(
                    operation,
                    BooleanOperation::Cut
                        | BooleanOperation::Union
                        | BooleanOperation::Intersect
                        | BooleanOperation::Split
                )
                .then(|| line_arc_capsule_profile_bounds(segments, true))
                .flatten()
            })
            .or_else(|| {
                (operation == BooleanOperation::Cut)
                    .then(|| strict_convex_line_arc_profile_bounds(segments, true))
                    .flatten()
            })
            .or_else(|| {
                matches!(
                    operation,
                    BooleanOperation::Cut
                        | BooleanOperation::Union
                        | BooleanOperation::Intersect
                        | BooleanOperation::Split
                )
                .then(|| line_segment_polygon_bounds(segments))
                .flatten()
            }) {
            Some(bounds) => bounds,
            None => return false,
        },
        _ => return false,
    };
    let base_width = target[2][0];
    let base_depth = target[2][1];
    let [tool_min_x, tool_min_y, tool_max_x, tool_max_y] = tool_bounds;
    match operation {
        BooleanOperation::Cut => {
            tool_min_x > 1.0e-6
                && tool_min_y > 1.0e-6
                && tool_max_x < base_width - 1.0e-6
                && tool_max_y < base_depth - 1.0e-6
                || matches!(
                    tool,
                    FeatureKind::SegmentProfile { segments, closed: true }
                        if line_arc_circle_side_overlap(
                            segments,
                            true,
                            base_width,
                            base_depth,
                        )
                        .is_some()
                            || line_arc_d_arc_only_side_overlap(
                            segments,
                            true,
                            base_width,
                            base_depth,
                        )
                        .is_some()
                            || line_arc_capsule_side_overlap(
                            segments,
                            true,
                            base_width,
                            base_depth,
                        )
                        .is_some()
                            || line_arc_capsule_corner_overlap(
                                segments,
                                true,
                                base_width,
                                base_depth,
                            )
                            .is_some()
                )
        }
        BooleanOperation::Intersect => {
            if let FeatureKind::SegmentProfile { segments, .. } = tool
                && tool_is_line_arc_d
            {
                let contained = tool_min_x > 1.0e-6
                    && tool_min_y > 1.0e-6
                    && tool_max_x < base_width - 1.0e-6
                    && tool_max_y < base_depth - 1.0e-6;
                return contained
                    || line_arc_d_arc_only_side_overlap(segments, true, base_width, base_depth)
                        .is_some();
            }
            if let FeatureKind::SegmentProfile { segments, .. } = tool
                && tool_is_capsule
            {
                return line_arc_capsule_side_overlap(segments, true, base_width, base_depth)
                    .is_some()
                    || line_arc_capsule_corner_overlap(segments, true, base_width, base_depth)
                        .is_some();
            }
            if let FeatureKind::SegmentProfile { segments, .. } = tool
                && (circle_segment_profile_bounds(segments).is_some()
                    || (line_segment_profile_bounds(segments).is_none()
                        && line_segment_polygon_bounds(segments).is_some()))
            {
                return tool_min_x > 1.0e-6
                    && tool_min_y > 1.0e-6
                    && tool_max_x < base_width - 1.0e-6
                    && tool_max_y < base_depth - 1.0e-6;
            }
            let overlap_x = base_width.min(tool_max_x) - 0.0_f64.max(tool_min_x);
            let overlap_y = base_depth.min(tool_max_y) - 0.0_f64.max(tool_min_y);
            overlap_x > 1.0e-6 && overlap_y > 1.0e-6
        }
        BooleanOperation::Split => {
            if let FeatureKind::SegmentProfile { segments, .. } = tool {
                if tool_is_line_arc_d {
                    let contained = tool_min_x > 1.0e-6
                        && tool_min_y > 1.0e-6
                        && tool_max_x < base_width - 1.0e-6
                        && tool_max_y < base_depth - 1.0e-6;
                    return contained
                        || line_arc_d_arc_only_side_overlap(
                            segments, true, base_width, base_depth,
                        )
                        .is_some();
                }
                if tool_is_capsule {
                    return line_arc_capsule_corner_overlap(segments, true, base_width, base_depth)
                        .is_some();
                }
                return (circle_segment_profile_bounds(segments).is_some()
                    || line_arc_d_profile_bounds(segments).is_some()
                    || line_segment_polygon_bounds(segments).is_some())
                    && tool_min_x > 1.0e-6
                    && tool_min_y > 1.0e-6
                    && tool_max_x < base_width - 1.0e-6
                    && tool_max_y < base_depth - 1.0e-6;
            }
            if !matches!(tool, FeatureKind::Profile { .. }) {
                return false;
            }
            let overlap_x = base_width.min(tool_max_x) - 0.0_f64.max(tool_min_x);
            let overlap_y = base_depth.min(tool_max_y) - 0.0_f64.max(tool_min_y);
            let boundary_crosses_target = (tool_min_x > 1.0e-6 && tool_min_x < base_width - 1.0e-6)
                || (tool_max_x > 1.0e-6 && tool_max_x < base_width - 1.0e-6)
                || (tool_min_y > 1.0e-6 && tool_min_y < base_depth - 1.0e-6)
                || (tool_max_y > 1.0e-6 && tool_max_y < base_depth - 1.0e-6);
            overlap_x > 1.0e-6 && overlap_y > 1.0e-6 && boundary_crosses_target
        }
        BooleanOperation::Union => {
            if let FeatureKind::SegmentProfile { segments, .. } = tool {
                if tool_is_capsule {
                    return line_arc_capsule_side_overlap(segments, true, base_width, base_depth)
                        .is_some()
                        || line_arc_capsule_corner_overlap(segments, true, base_width, base_depth)
                            .is_some();
                }
                if tool_is_line_arc_d {
                    return line_arc_d_arc_only_side_overlap(
                        segments, true, base_width, base_depth,
                    )
                    .is_some()
                        || line_arc_d_profile_contains_rectangle(segments, base_width, base_depth)
                            && (tool_min_x < -1.0e-6
                                || tool_min_y < -1.0e-6
                                || tool_max_x > base_width + 1.0e-6
                                || tool_max_y > base_depth + 1.0e-6);
                }
                if let Some([min_x, min_y, max_x, max_y]) = circle_segment_profile_bounds(segments)
                {
                    let center = [(min_x + max_x) * 0.5, (min_y + max_y) * 0.5];
                    let radius = (max_x - min_x) * 0.5;
                    return [
                        [0.0, 0.0],
                        [base_width, 0.0],
                        [base_width, base_depth],
                        [0.0, base_depth],
                    ]
                    .into_iter()
                    .all(|corner| {
                        (corner[0] - center[0]).hypot(corner[1] - center[1]) < radius - 1.0e-6
                    });
                }
                return line_segment_polygon_contains_rectangle(segments, base_width, base_depth)
                    && (tool_min_x < -1.0e-6
                        || tool_min_y < -1.0e-6
                        || tool_max_x > base_width + 1.0e-6
                        || tool_max_y > base_depth + 1.0e-6);
            }
            let overlap_x = base_width.min(tool_max_x) - 0.0_f64.max(tool_min_x);
            let overlap_y = base_depth.min(tool_max_y) - 0.0_f64.max(tool_min_y);
            if overlap_x <= 1.0e-6 || overlap_y <= 1.0e-6 {
                return false;
            }
            let bounds = [
                0.0_f64.min(tool_min_x),
                0.0_f64.min(tool_min_y),
                base_width.max(tool_max_x),
                base_depth.max(tool_max_y),
            ];
            let union_area = base_width * base_depth
                + (tool_max_x - tool_min_x) * (tool_max_y - tool_min_y)
                - overlap_x * overlap_y;
            let bounds_area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
            let tolerance = 1.0e-6_f64.max(bounds_area.abs() * 1.0e-10);
            (union_area - bounds_area).abs() <= tolerance
                && (bounds[0] < -1.0e-6
                    || bounds[1] < -1.0e-6
                    || bounds[2] > base_width + 1.0e-6
                    || bounds[3] > base_depth + 1.0e-6)
        }
    }
}

fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, CanonicalError> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(CanonicalError::IdExhausted)
}

fn group_is_descendant(product: &ProductModel, root: GroupId, target: GroupId) -> bool {
    let mut cursor = Some(target);
    while let Some(candidate) = cursor {
        if candidate == root {
            return true;
        }
        cursor = product
            .groups
            .get(&candidate)
            .and_then(|group| group.parent);
    }
    false
}

fn descendant_groups(
    product: &ProductModel,
    root: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    if !product.groups.contains_key(&root) {
        return Err(CanonicalError::GroupNotFound(root));
    }
    Ok(product
        .groups
        .keys()
        .cloned()
        .filter(|id| group_is_descendant(product, root, *id))
        .collect())
}

fn world_group_lineage(
    product: &ProductModel,
    target: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    let mut lineage = Vec::new();
    let mut cursor = Some(target);
    while let Some(id) = cursor {
        let group = product
            .groups
            .get(&id)
            .ok_or(CanonicalError::GroupNotFound(id))?;
        lineage.push(id);
        cursor = group.parent;
    }
    lineage.reverse();
    Ok(lineage)
}

fn group_lineage(
    product: &ProductModel,
    root: GroupId,
    target: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    let mut lineage = Vec::new();
    let mut cursor = Some(target);
    while let Some(id) = cursor {
        lineage.push(id);
        if id == root {
            lineage.reverse();
            return Ok(lineage);
        }
        cursor = product.groups.get(&id).and_then(|group| group.parent);
    }
    Err(CanonicalError::InvalidLocalGraph)
}

fn conversion_mappings(
    product: &ProductModel,
    plan: &ConvertGroupPlan,
) -> Result<Vec<ConversionMapping>, CanonicalError> {
    let converted_groups = descendant_groups(product, plan.group_id)?;
    let converted_group_set: BTreeSet<_> = converted_groups.iter().cloned().collect();
    let mut mappings = Vec::new();
    for id in product.groups.keys().cloned() {
        let old_path = WorldEntityPath {
            groups: world_group_lineage(product, id)?,
            occurrence: None,
        };
        let resolution = if converted_group_set.contains(&id) {
            let converted_lineage = group_lineage(product, plan.group_id, id)?;
            let mut new_path = InstancePath::root(plan.new_occurrence_id);
            new_path.steps.extend(
                converted_lineage
                    .iter()
                    .skip(1)
                    .map(|id| InstancePathStep::Group(LocalGroupId(id.0))),
            );
            MappingResolution::Resolved {
                new_id: if id == plan.group_id {
                    ConvertedEntityId::ComponentOccurrence(plan.new_occurrence_id)
                } else {
                    ConvertedEntityId::LocalGroup(LocalGroupKey {
                        definition_id: plan.new_definition_id,
                        local_id: LocalGroupId(id.0),
                    })
                },
                new_path,
            }
        } else {
            MappingResolution::Unresolved {
                reason: UnresolvedMappingReason::NotInConvertedGroup,
            }
        };
        mappings.push(ConversionMapping {
            old_id: WorldEntityId::Group(id),
            old_path,
            resolution,
        });
    }
    for occurrence in product.occurrences.values() {
        let old_groups = occurrence.parent.map_or_else(
            || Ok(Vec::new()),
            |parent| world_group_lineage(product, parent),
        )?;
        let old_path = WorldEntityPath {
            groups: old_groups,
            occurrence: Some(occurrence.id),
        };
        let resolution = if occurrence
            .parent
            .is_some_and(|parent| converted_group_set.contains(&parent))
        {
            let converted_lineage =
                group_lineage(product, plan.group_id, occurrence.parent.unwrap())?;
            let mut new_path = InstancePath::root(plan.new_occurrence_id);
            new_path.steps.extend(
                converted_lineage
                    .iter()
                    .skip(1)
                    .map(|id| InstancePathStep::Group(LocalGroupId(id.0))),
            );
            new_path
                .steps
                .push(InstancePathStep::Occurrence(LocalOccurrenceId(
                    occurrence.id.0,
                )));
            MappingResolution::Resolved {
                new_id: ConvertedEntityId::LocalOccurrence(LocalOccurrenceKey {
                    definition_id: plan.new_definition_id,
                    local_id: LocalOccurrenceId(occurrence.id.0),
                }),
                new_path,
            }
        } else {
            MappingResolution::Unresolved {
                reason: UnresolvedMappingReason::NotInConvertedGroup,
            }
        };
        mappings.push(ConversionMapping {
            old_id: WorldEntityId::Occurrence(occurrence.id),
            old_path,
            resolution,
        });
    }
    mappings.sort_by(|left, right| left.old_path.cmp(&right.old_path));
    Ok(mappings)
}

fn convert_group_to_component_model(
    product: &mut ProductModel,
    plan: &ConvertGroupPlan,
) -> Result<(), CanonicalError> {
    ensure_name(&plan.component_name)?;
    if product.definitions.contains_key(&plan.new_definition_id) {
        return Err(CanonicalError::DefinitionAlreadyExists(
            plan.new_definition_id,
        ));
    }
    if product.occurrences.contains_key(&plan.new_occurrence_id) {
        return Err(CanonicalError::OccurrenceAlreadyExists(
            plan.new_occurrence_id,
        ));
    }
    let root = product
        .groups
        .get(&plan.group_id)
        .ok_or(CanonicalError::GroupNotFound(plan.group_id))?
        .as_ref()
        .clone();
    let groups = descendant_groups(product, plan.group_id)?;
    let group_set: BTreeSet<_> = groups.iter().cloned().collect();
    let occurrence_ids: Vec<_> = product
        .occurrences
        .values()
        .filter(|item| {
            item.parent
                .is_some_and(|parent| group_set.contains(&parent))
        })
        .map(|item| item.id)
        .collect();
    let local_group_ids = groups
        .iter()
        .cloned()
        .filter(|id| *id != plan.group_id)
        .map(|id| LocalGroupId(id.0))
        .collect::<Vec<_>>();
    let local_occurrence_ids = occurrence_ids
        .iter()
        .map(|id| LocalOccurrenceId(id.0))
        .collect::<Vec<_>>();
    product.definitions.insert(
        plan.new_definition_id,
        Arc::new(Definition {
            local_occurrence_ids: local_occurrence_ids.clone(),
            local_group_ids: local_group_ids.clone(),
            ..new_definition(plan.new_definition_id, plan.component_name.clone())
        }),
    );
    for id in groups.iter().cloned().filter(|id| *id != plan.group_id) {
        let group = product.groups[&id].as_ref();
        let key = LocalGroupKey {
            definition_id: plan.new_definition_id,
            local_id: LocalGroupId(id.0),
        };
        product.local_groups.insert(
            key,
            Arc::new(LocalGroup {
                key,
                name: group.name.clone(),
                transform: group.transform,
                parent: group
                    .parent
                    .filter(|parent| *parent != plan.group_id)
                    .map(|parent| LocalGroupId(parent.0)),
            }),
        );
    }
    for id in &occurrence_ids {
        let occurrence = product.occurrences[id].as_ref();
        let key = LocalOccurrenceKey {
            definition_id: plan.new_definition_id,
            local_id: LocalOccurrenceId(id.0),
        };
        product.local_occurrences.insert(
            key,
            Arc::new(LocalOccurrence {
                key,
                definition_id: occurrence.definition_id,
                name: occurrence.name.clone(),
                transform: occurrence.transform,
                parent: occurrence
                    .parent
                    .filter(|parent| *parent != plan.group_id)
                    .map(|parent| LocalGroupId(parent.0)),
                tag: occurrence.tag,
                visible: occurrence.visible,
            }),
        );
    }
    for id in occurrence_ids {
        product.occurrences.remove(&id);
    }
    for id in groups {
        product.groups.remove(&id);
    }
    product.occurrences.insert(
        plan.new_occurrence_id,
        Arc::new(Occurrence {
            id: plan.new_occurrence_id,
            definition_id: plan.new_definition_id,
            name: plan.component_name.clone(),
            transform: root.transform,
            parent: root.parent,
            tag: None,
            visible: true,
        }),
    );
    Ok(())
}

fn refresh_override_health(product: &mut ProductModel) {
    for value in product.overrides.values_mut() {
        let audited = resolve_derived_identity(&product.evaluator_nodes, &value.target);
        if value.health != audited {
            let mut refreshed = value.as_ref().clone();
            refreshed.health = audited;
            *value = Arc::new(refreshed);
        }
    }
}

fn validate_overrides(product: &ProductModel) -> Result<(), CanonicalError> {
    for value in product.overrides.values() {
        if let Some(root) = product.evaluator_nodes.get(&value.target.root_rule_node_id)
            && !root
                .allowed_parameters()
                .iter()
                .any(|parameter| parameter.name() == value.parameter)
        {
            return Err(CanonicalError::UndeclaredOverrideParameter);
        }
    }
    Ok(())
}

fn supported_planar_face_frame(
    product: &ProductModel,
    reference: &BodySubshapeRef,
) -> Option<WorkplaneFrame> {
    let snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    let request = ExactFeatureChainRequest::from_snapshot_for_producer(
        &snapshot,
        reference.definition_id,
        reference.producer_feature_id,
    )
    .ok()?;
    if !reference.matches_durable_request_identity(&request) {
        return None;
    }
    let width_mm = f64::from_bits(request.width_bits);
    let height_mm = f64::from_bits(request.height_bits);
    if let Some(frame_bits) = request.workplane_frame_bits {
        let frame = frame_bits.map(f64::from_bits);
        let origin = [frame[0], frame[1], frame[2]];
        let x_axis = [frame[3], frame[4], frame[5]];
        let y_axis = [frame[6], frame[7], frame[8]];
        let normal = [frame[9], frame[10], frame[11]];
        let cross_xy = [
            x_axis[1] * y_axis[2] - x_axis[2] * y_axis[1],
            x_axis[2] * y_axis[0] - x_axis[0] * y_axis[2],
            x_axis[0] * y_axis[1] - x_axis[1] * y_axis[0],
        ];
        let right_handed =
            cross_xy[0] * normal[0] + cross_xy[1] * normal[1] + cross_xy[2] * normal[2] > 0.0;
        let negate = |axis: [f64; 3]| [-axis[0], -axis[1], -axis[2]];
        let translated = |axis: [f64; 3], distance: f64| {
            [
                origin[0] + axis[0] * distance,
                origin[1] + axis[1] * distance,
                origin[2] + axis[2] * distance,
            ]
        };
        return match reference.role()? {
            ExactFaceRole::Top => Some(WorkplaneFrame {
                origin_mm: translated(normal, height_mm),
                x_axis: if right_handed { x_axis } else { negate(x_axis) },
                y_axis,
                normal,
            }),
            ExactFaceRole::Bottom => Some(WorkplaneFrame {
                origin_mm: origin,
                x_axis,
                y_axis: if right_handed { negate(y_axis) } else { y_axis },
                normal: negate(normal),
            }),
            ExactFaceRole::East
                if request.boolean.is_none()
                    && request.shell.is_none()
                    && request.pocket_depth_bits.is_none() =>
            {
                Some(WorkplaneFrame {
                    origin_mm: translated(x_axis, width_mm),
                    x_axis: y_axis,
                    y_axis: if right_handed { normal } else { negate(normal) },
                    normal: x_axis,
                })
            }
            _ => None,
        };
    }
    match reference.role()? {
        ExactFaceRole::Top => Some(WorkplaneFrame::principal(PrincipalPlane::Xy).offset(height_mm)),
        ExactFaceRole::Bottom => Some(WorkplaneFrame {
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, -1.0, 0.0],
            normal: [0.0, 0.0, -1.0],
        }),
        ExactFaceRole::East
            if request.boolean.is_none()
                && request.shell.is_none()
                && request.pocket_depth_bits.is_none() =>
        {
            Some(WorkplaneFrame {
                origin_mm: [width_mm, 0.0, 0.0],
                x_axis: [0.0, 1.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
                normal: [1.0, 0.0, 0.0],
            })
        }
        _ => None,
    }
}

fn set_planar_face_reference_health(
    product: &mut ProductModel,
    lineage_digest: &str,
    health: WorkplaneSupportHealth,
) {
    let anchored = product
        .features
        .values()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::PlanarFace { reference, .. },
                ..
            }) if reference.lineage_digest == lineage_digest => Some(feature.id),
            _ => None,
        })
        .collect::<Vec<_>>();
    for id in anchored {
        let feature = Arc::clone(
            product
                .features
                .get(&id)
                .expect("collected workplane exists"),
        );
        let FeatureKind::Workplane(spec) = &feature.kind else {
            unreachable!("collected feature is a workplane");
        };
        let WorkplaneSupport::PlanarFace { reference, .. } = &spec.support else {
            unreachable!("collected workplane has planar-face support");
        };
        let mut updated = spec.clone();
        updated.support = WorkplaneSupport::PlanarFace {
            reference: reference.clone(),
            health,
        };
        product.features.insert(
            id,
            Arc::new(Feature {
                kind: FeatureKind::Workplane(updated),
                ..feature.as_ref().clone()
            }),
        );
    }
}

fn rebind_planar_face_reference(
    product: &mut ProductModel,
    reference: &BodySubshapeRef,
) -> Result<(), CanonicalError> {
    let anchored = product
        .features
        .values()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::Workplane(WorkplaneSpec {
                support:
                    WorkplaneSupport::PlanarFace {
                        reference: support, ..
                    },
                ..
            }) if support.lineage_digest == reference.lineage_digest => Some(feature.id),
            _ => None,
        })
        .collect::<Vec<_>>();
    for id in anchored {
        let feature = Arc::clone(
            product
                .features
                .get(&id)
                .expect("collected workplane exists"),
        );
        let FeatureKind::Workplane(spec) = &feature.kind else {
            unreachable!("collected feature is a workplane");
        };
        let mut updated = spec.clone();
        updated.support = WorkplaneSupport::PlanarFace {
            reference: Box::new(reference.clone()),
            health: WorkplaneSupportHealth::Resolved,
        };
        product.features.insert(
            id,
            Arc::new(Feature {
                kind: FeatureKind::Workplane(updated),
                ..feature.as_ref().clone()
            }),
        );
    }
    let dependent_pockets = product
        .features
        .values()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::SketchPocket(spec)
                if spec.support.lineage_digest == reference.lineage_digest =>
            {
                Some(feature.id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for id in dependent_pockets {
        let feature = Arc::clone(product.features.get(&id).expect("collected Pocket exists"));
        let FeatureKind::SketchPocket(spec) = &feature.kind else {
            unreachable!("collected feature is a Pocket");
        };
        let mut updated = spec.clone();
        updated.support = Box::new(reference.clone());
        product.features.insert(
            id,
            Arc::new(Feature {
                kind: FeatureKind::SketchPocket(updated),
                ..feature.as_ref().clone()
            }),
        );
    }
    refresh_supported_planar_face_frames(product, None)
}

fn refresh_supported_planar_face_frames(
    product: &mut ProductModel,
    previous: Option<&Snapshot>,
) -> Result<(), CanonicalError> {
    let updates = product
        .features
        .values()
        .filter_map(|feature| {
            let FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::PlanarFace { reference, .. },
                ..
            }) = &feature.kind
            else {
                return None;
            };
            if let Some(previous) = previous {
                let anchor_existed = matches!(
                    previous.feature(feature.id).map(Feature::kind),
                    Some(FeatureKind::Workplane(WorkplaneSpec {
                        support: WorkplaneSupport::PlanarFace { reference: prior, .. },
                        ..
                    })) if prior.lineage_digest == reference.lineage_digest
                );
                let producer_changed = previous
                    .feature(reference.producer_feature_id)
                    .zip(product.features.get(&reference.producer_feature_id))
                    .is_some_and(|(before, after)| before.kind() != &after.kind);
                if !anchor_existed || !producer_changed {
                    return None;
                }
            }
            Some((
                feature.id,
                supported_planar_face_frame(product, reference).ok_or(CanonicalError::Sketch(
                    SketchError::InvalidPlanarFaceSupport,
                )),
            ))
        })
        .collect::<Vec<_>>();
    for (id, frame) in updates {
        let frame = frame?;
        let feature = Arc::clone(
            product
                .features
                .get(&id)
                .expect("collected workplane exists"),
        );
        let FeatureKind::Workplane(spec) = &feature.kind else {
            unreachable!("collected feature is a workplane");
        };
        let mut updated = spec.clone();
        updated.frame = frame;
        product.features.insert(
            id,
            Arc::new(Feature {
                kind: FeatureKind::Workplane(updated),
                ..feature.as_ref().clone()
            }),
        );
    }
    Ok(())
}

fn validate_assembly_mate(
    product: &ProductModel,
    mate: &AssemblyMate,
    require_resolved: bool,
) -> Result<(), CanonicalError> {
    if mate.schema() != ASSEMBLY_MATE_SCHEMA_V1
        || mate.id().0 == 0
        || !mate.kind().is_valid()
        || mate.endpoint_a().occurrence_id() == mate.endpoint_b().occurrence_id()
    {
        return Err(CanonicalError::InvalidAssemblyMate(mate.id()));
    }
    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
        let occurrence = product
            .occurrences
            .get(&endpoint.occurrence_id())
            .ok_or(CanonicalError::OccurrenceNotFound(endpoint.occurrence_id()))?;
        let reference = endpoint.reference();
        let health_is_valid = match endpoint.health() {
            AssemblyReferenceHealth::Resolved => true,
            AssemblyReferenceHealth::Broken | AssemblyReferenceHealth::Lost => !require_resolved,
            AssemblyReferenceHealth::Ambiguous { candidate_count } => {
                !require_resolved && candidate_count > 1
            }
        };
        if !health_is_valid
            || !reference.has_valid_lineage()
            || reference.document_id != product.document_id
            || reference.definition_id != occurrence.definition_id
            || product
                .features
                .get(&reference.profile_feature_id)
                .is_none_or(|feature| feature.definition_id != occurrence.definition_id)
            || product
                .features
                .get(&reference.producer_feature_id)
                .is_none_or(|feature| feature.definition_id != occurrence.definition_id)
        {
            return Err(CanonicalError::InvalidAssemblyMate(mate.id()));
        }
    }
    let type_is_valid = match mate.kind() {
        AssemblyMateKind::CoincidentPlanar { .. }
        | AssemblyMateKind::Distance { .. }
        | AssemblyMateKind::Angle { .. } => [mate.endpoint_a(), mate.endpoint_b()]
            .iter()
            .all(|endpoint| endpoint.reference().expected_type == "planar_face"),
        AssemblyMateKind::ConcentricAxial { .. } => [mate.endpoint_a(), mate.endpoint_b()]
            .iter()
            .all(|endpoint| {
                endpoint.reference().expected_type.ends_with("_face")
                    || endpoint.reference().expected_type.ends_with("_edge")
                    || matches!(endpoint.reference().expected_type.as_str(), "face" | "edge")
            }),
    };
    if !type_is_valid {
        return Err(CanonicalError::InvalidAssemblyMate(mate.id()));
    }
    Ok(())
}

fn validate_assembly_joint(
    product: &ProductModel,
    joint: &AssemblyJoint,
) -> Result<(), CanonicalError> {
    if !joint.has_valid_shape()
        || joint.schema() != ASSEMBLY_JOINT_SCHEMA_V1
        || !product
            .occurrences
            .contains_key(&joint.parent_occurrence_id())
        || !product
            .occurrences
            .contains_key(&joint.child_occurrence_id())
        || product.assembly_joints.values().any(|existing| {
            existing.id() != joint.id()
                && existing.child_occurrence_id() == joint.child_occurrence_id()
        })
    {
        return Err(CanonicalError::InvalidAssemblyJoint(joint.id()));
    }

    let mut parent_by_child = product
        .assembly_joints
        .values()
        .filter(|existing| existing.id() != joint.id())
        .map(|existing| {
            (
                existing.child_occurrence_id(),
                existing.parent_occurrence_id(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    parent_by_child.insert(joint.child_occurrence_id(), joint.parent_occurrence_id());
    let mut cursor = joint.parent_occurrence_id();
    let mut visited = BTreeSet::new();
    while let Some(parent) = parent_by_child.get(&cursor).copied() {
        if parent == joint.child_occurrence_id() || !visited.insert(cursor) {
            return Err(CanonicalError::InvalidAssemblyJoint(joint.id()));
        }
        cursor = parent;
    }
    Ok(())
}

fn validate_assembly_motion_coupling(
    product: &ProductModel,
    coupling: &AssemblyMotionCoupling,
) -> Result<(), CanonicalError> {
    if !coupling.has_valid_shape() || coupling.schema() != ASSEMBLY_MOTION_COUPLING_SCHEMA_V1 {
        return Err(CanonicalError::InvalidAssemblyMotionCoupling(coupling.id()));
    }
    let input = product
        .assembly_joints
        .get(&coupling.input_joint_id())
        .ok_or(CanonicalError::AssemblyJointNotFound(
            coupling.input_joint_id(),
        ))?;
    let output = product
        .assembly_joints
        .get(&coupling.output_joint_id())
        .ok_or(CanonicalError::AssemblyJointNotFound(
            coupling.output_joint_id(),
        ))?;
    let (expected_input, expected_output) = coupling.transmission().joint_kinds();
    if joint_kind_class(input.kind()) != Some(expected_input)
        || joint_kind_class(output.kind()) != Some(expected_output)
        || input
            .kind()
            .with_position(coupling.input_reference_position())
            .is_none_or(|kind| !kind.is_valid())
        || output
            .kind()
            .with_position(coupling.output_reference_position())
            .is_none_or(|kind| !kind.is_valid())
        || !motion_values_equal(
            output
                .kind()
                .position()
                .expect("typed coupling output is movable"),
            coupling.output_position(
                input
                    .kind()
                    .position()
                    .expect("typed coupling input is movable"),
            ),
        )
    {
        return Err(CanonicalError::InvalidAssemblyMotionCoupling(coupling.id()));
    }

    let mut couplings = product
        .assembly_motion_couplings
        .values()
        .filter(|existing| existing.id() != coupling.id())
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    couplings.push(coupling);
    validate_motion_coupling_graph(&couplings)
        .map_err(CanonicalError::InvalidAssemblyMotionCoupling)
}

fn joint_kind_class(kind: AssemblyJointKind) -> Option<CoupledJointKind> {
    match kind {
        AssemblyJointKind::Fixed => None,
        AssemblyJointKind::Revolute { .. } => Some(CoupledJointKind::Revolute),
        AssemblyJointKind::Prismatic { .. } => Some(CoupledJointKind::Prismatic),
    }
}

fn validate_motion_coupling_graph(
    couplings: &[&AssemblyMotionCoupling],
) -> Result<(), AssemblyMotionCouplingId> {
    let mut adjacency = BTreeMap::<
        AssemblyJointId,
        Vec<(AssemblyJointId, f64, f64, AssemblyMotionCouplingId)>,
    >::new();
    for coupling in couplings {
        let scale = coupling.transmission().scale();
        let offset =
            coupling.output_reference_position() - scale * coupling.input_reference_position();
        adjacency
            .entry(coupling.input_joint_id())
            .or_default()
            .push((coupling.output_joint_id(), scale, offset, coupling.id()));
        adjacency
            .entry(coupling.output_joint_id())
            .or_default()
            .push((
                coupling.input_joint_id(),
                scale.recip(),
                -offset / scale,
                coupling.id(),
            ));
    }

    let mut unresolved = adjacency.keys().copied().collect::<BTreeSet<_>>();
    while let Some(root) = unresolved.pop_first() {
        let mut transforms = BTreeMap::from([(root, (1.0_f64, 0.0_f64))]);
        let mut pending = vec![root];
        while let Some(joint_id) = pending.pop() {
            let (source_scale, source_offset) = transforms[&joint_id];
            for (neighbour, edge_scale, edge_offset, coupling_id) in
                adjacency.get(&joint_id).into_iter().flatten()
            {
                let candidate = (
                    edge_scale * source_scale,
                    edge_scale * source_offset + edge_offset,
                );
                if !candidate.0.is_finite()
                    || candidate.0 == 0.0
                    || !candidate.0.recip().is_finite()
                    || !candidate.1.is_finite()
                    || !(-candidate.1 / candidate.0).is_finite()
                {
                    return Err(*coupling_id);
                }
                if let Some(existing) = transforms.get(neighbour) {
                    if !motion_values_equal(existing.0, candidate.0)
                        || !motion_values_equal(existing.1, candidate.1)
                    {
                        return Err(*coupling_id);
                    }
                } else {
                    transforms.insert(*neighbour, candidate);
                    unresolved.remove(neighbour);
                    pending.push(*neighbour);
                }
            }
        }
    }
    Ok(())
}

fn motion_values_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= 1.0e-10 * scale
}

fn validate_assembly_motion_study(
    product: &ProductModel,
    study: &AssemblyMotionStudy,
) -> Result<(), CanonicalError> {
    if !study.has_valid_shape() || study.schema() != ASSEMBLY_MOTION_STUDY_SCHEMA_V1 {
        return Err(CanonicalError::InvalidAssemblyMotionStudy(study.id()));
    }
    ensure_name(study.name())?;
    for driver in study.drivers() {
        let joint = product
            .assembly_joints
            .get(&driver.joint_id())
            .ok_or(CanonicalError::AssemblyJointNotFound(driver.joint_id()))?;
        if joint
            .kind()
            .with_position(driver.position())
            .is_none_or(|kind| !kind.is_valid())
        {
            return Err(CanonicalError::InvalidAssemblyMotionStudy(study.id()));
        }
    }
    Ok(())
}

fn validate_drawing_sheet(
    product: &ProductModel,
    sheet: &DrawingSheet,
) -> Result<(), CanonicalError> {
    ensure_product_id(sheet.id().0)?;
    ensure_name(sheet.name())?;
    if sheet.schema() != crate::drawing::ORTHOGRAPHIC_DRAWING_SCHEMA_V1 {
        return Err(CanonicalError::Drawing(DrawingError::InvalidSheet));
    }
    let snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    crate::drawing::validate_source(&snapshot, sheet.source()).map_err(CanonicalError::Drawing)
}

fn validate_assembly_joint_motion_publication(
    current: &Snapshot,
    product: &ProductModel,
    batch: &CommandBatch,
) -> Result<(), CanonicalError> {
    let final_joints_by_child = product
        .assembly_joints
        .values()
        .map(|joint| (joint.child_occurrence_id(), joint.as_ref()))
        .collect::<BTreeMap<_, _>>();
    for before in current.assembly_joints() {
        if let Some(after) = product.assembly_joints.get(&before.id())
            && (before.parent_occurrence_id() != after.parent_occurrence_id()
                || before.child_occurrence_id() != after.child_occurrence_id())
        {
            return Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
                before.id(),
            ));
        }
        if let Some(after) = final_joints_by_child.get(&before.child_occurrence_id())
            && after.id() != before.id()
        {
            return Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
                before.id(),
            ));
        }
    }

    let kind_overrides = current
        .assembly_joints()
        .filter_map(|before| {
            product
                .assembly_joints
                .get(&before.id())
                .filter(|after| !joint_motion_states_equal(before.kind(), after.kind()))
                .map(|after| (before.id(), after.kind()))
        })
        .collect::<BTreeMap<_, _>>();
    let Some(first_changed_joint_id) = kind_overrides.keys().next().copied() else {
        return Ok(());
    };

    let expected_solution =
        solve_assembly_joint_kinematics_with_kind_overrides(current, &kind_overrides)
            .map_err(|_| CanonicalError::InvalidAssemblySolvePublication)?;
    let required_transform_ids = kind_overrides
        .keys()
        .filter_map(|id| {
            current
                .assembly_joint(*id)
                .map(AssemblyJoint::child_occurrence_id)
        })
        .collect::<BTreeSet<_>>();
    let expected_transforms = expected_solution
        .poses()
        .iter()
        .filter_map(|pose| {
            current
                .occurrence(pose.occurrence_id())
                .filter(|occurrence| {
                    required_transform_ids.contains(&pose.occurrence_id())
                        || !transforms_equivalent(occurrence.transform(), pose.local_transform())
                })
                .map(|_| (pose.occurrence_id(), pose.local_transform()))
        })
        .collect::<Vec<_>>();
    let solve_publications = batch
        .commands
        .iter()
        .filter_map(|command| match command {
            CanonicalCommand::ApplyAssemblySolve { transforms, .. } => Some(transforms),
            _ => None,
        })
        .collect::<Vec<_>>();
    if solve_publications.len() != 1
        || solve_publications[0].len() != expected_transforms.len()
        || solve_publications[0].iter().zip(&expected_transforms).any(
            |((actual_id, actual), (expected_id, expected))| {
                actual_id != expected_id || !transforms_equivalent(*actual, *expected)
            },
        )
        || expected_transforms.iter().any(|(id, expected)| {
            product
                .occurrences
                .get(id)
                .is_none_or(|occurrence| !transforms_equivalent(occurrence.transform(), *expected))
        })
    {
        return Err(CanonicalError::UnsynchronizedAssemblyJointPosition(
            first_changed_joint_id,
        ));
    }
    Ok(())
}

fn validate_product(product: &ProductModel) -> Result<(), CanonicalError> {
    ensure_product_id(product.document_id.0)?;
    if let Some(id) = product
        .grounded_occurrences
        .iter()
        .find(|id| !product.occurrences.contains_key(id))
    {
        return Err(CanonicalError::OccurrenceNotFound(*id));
    }
    for (id, mate) in &product.assembly_mates {
        if *id != mate.id() {
            return Err(CanonicalError::InvalidAssemblyMate(*id));
        }
        validate_assembly_mate(product, mate, false)?;
    }
    for (id, joint) in &product.assembly_joints {
        if *id != joint.id() {
            return Err(CanonicalError::InvalidAssemblyJoint(*id));
        }
        validate_assembly_joint(product, joint)?;
    }
    for (id, coupling) in &product.assembly_motion_couplings {
        if *id != coupling.id() {
            return Err(CanonicalError::InvalidAssemblyMotionCoupling(*id));
        }
        validate_assembly_motion_coupling(product, coupling)?;
    }
    for (id, study) in &product.assembly_motion_studies {
        if *id != study.id() {
            return Err(CanonicalError::InvalidAssemblyMotionStudy(*id));
        }
        validate_assembly_motion_study(product, study)?;
    }
    for (id, sheet) in &product.drawing_sheets {
        if *id != sheet.id() {
            return Err(CanonicalError::Drawing(DrawingError::InvalidSheet));
        }
        validate_drawing_sheet(product, sheet)?;
    }
    FeatureDependencyGraph::from_product(product)?;
    for (id, joint) in &product.joints {
        if *id != joint.id() || !joint.volume().has_positive_volume() {
            return Err(CanonicalError::Prismatic(PrismaticError::EmptyVolume));
        }
    }
    for (id, space) in &product.spaces {
        if *id != space.id() || !space.volume().has_positive_volume() {
            return Err(CanonicalError::Space(SpaceError::InvalidVolume));
        }
        for adjacent_id in space.adjacent_to() {
            let adjacent = product
                .spaces
                .get(adjacent_id)
                .ok_or(CanonicalError::Space(SpaceError::MissingSpace))?;
            if !adjacent.adjacent_to().contains(id) {
                return Err(CanonicalError::Space(SpaceError::AsymmetricAdjacency));
            }
        }
        if space
            .accessible_to()
            .iter()
            .any(|target| !product.spaces.contains_key(target))
        {
            return Err(CanonicalError::Space(SpaceError::MissingSpace));
        }
    }
    let validation_snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    for (id, clearance) in &product.clearance_volumes {
        if *id != clearance.id() || !clearance.volume().has_positive_volume() {
            return Err(CanonicalError::Space(SpaceError::InvalidVolume));
        }
        let owner_is_valid = match clearance.owner() {
            ClearanceOwner::Occurrence(path) => {
                validation_snapshot.resolve_instance_path(path).is_ok()
            }
            ClearanceOwner::Space(space_id) => product.spaces.contains_key(space_id),
        };
        if !owner_is_valid {
            return Err(CanonicalError::Space(SpaceError::InvalidOwner));
        }
    }
    for (id, dimension) in &product.persistent_dimensions {
        if *id != dimension.id {
            return Err(CanonicalError::InvalidPersistentDimensionTarget);
        }
        validate_persistent_dimension(dimension)?;
    }
    for tag in product.tags.values() {
        ensure_product_id(tag.id.0)?;
        ensure_name(&tag.name)?;
    }
    for (id, collection) in &product.collections {
        if *id != collection.id {
            return Err(CanonicalError::CollectionNotFound(*id));
        }
        ensure_product_id(collection.id.0)?;
        ensure_name(&collection.name)?;
        for occurrence_id in &collection.occurrence_ids {
            if !product.occurrences.contains_key(occurrence_id) {
                return Err(CanonicalError::OccurrenceNotFound(*occurrence_id));
            }
        }
    }
    for (id, receipt) in &product.import_receipts {
        if *id != receipt.id() || receipt.validate().is_err() {
            return Err(CanonicalError::InvalidImportReceipt);
        }
    }
    for definition in product.definitions.values() {
        ensure_product_id(definition.id.0)?;
        ensure_name(&definition.name)?;
        if definition.bodies.is_empty()
            || !definition.bodies.contains_key(&definition.active_body_id)
        {
            return Err(CanonicalError::BodyNotFound(
                definition.id,
                definition.active_body_id,
            ));
        }
        for (id, body) in &definition.bodies {
            if *id != body.id {
                return Err(CanonicalError::BodyNotFound(definition.id, *id));
            }
            ensure_product_id(body.id.0)?;
            ensure_name(&body.name)?;
            if let Some(feature_id) = body.consumed_by {
                let feature = product
                    .features
                    .get(&feature_id)
                    .ok_or(CanonicalError::InvalidBodyAuthoringPlan)?;
                let ownership = definition
                    .feature_body_ownership
                    .get(&feature_id)
                    .ok_or(CanonicalError::InvalidBodyAuthoringPlan)?;
                if feature.definition_id != definition.id
                    || !matches!(feature.kind, FeatureKind::Boolean { .. })
                    || !ownership.input_body_ids.contains(id)
                    || ownership.output_body_id == Some(*id)
                    || definition.active_body_id == *id
                {
                    return Err(CanonicalError::InvalidBodyAuthoringPlan);
                }
            }
        }
        let mut seen = BTreeSet::new();
        for feature_id in &definition.feature_ids {
            if !seen.insert(*feature_id) {
                return Err(CanonicalError::InvalidFeatureOwnership(*feature_id));
            }
            let feature = product
                .features
                .get(feature_id)
                .ok_or(CanonicalError::FeatureNotFound(*feature_id))?;
            if feature.definition_id != definition.id {
                return Err(CanonicalError::InvalidFeatureOwnership(*feature_id));
            }
            let ownership = definition
                .feature_body_ownership
                .get(feature_id)
                .ok_or(CanonicalError::InvalidBodyOwnership(*feature_id))?;
            if ownership
                .input_body_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
                || ownership
                    .input_body_ids
                    .iter()
                    .chain(ownership.output_body_id.iter())
                    .any(|id| !definition.bodies.contains_key(id))
                || feature_kind_is_solid(&feature.kind) != ownership.output_body_id.is_some()
                || inferred_feature_body_ownership(product, definition, &feature.kind)?
                    .input_body_ids
                    != ownership.input_body_ids
            {
                return Err(CanonicalError::InvalidBodyOwnership(*feature_id));
            }
        }
        if definition.feature_body_ownership.len() != definition.feature_ids.len() {
            return Err(CanonicalError::InvalidFeatureOwnership(
                definition
                    .feature_body_ownership
                    .keys()
                    .find(|id| !seen.contains(id))
                    .cloned()
                    .unwrap_or(FeatureId(0)),
            ));
        }
        validate_body_dependency_graph(definition)?;
        let mut local_ids = BTreeSet::new();
        for local_id in &definition.local_group_ids {
            if !local_ids.insert(local_id.0)
                || !product.local_groups.contains_key(&LocalGroupKey {
                    definition_id: definition.id,
                    local_id: *local_id,
                })
            {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
        for local_id in &definition.local_occurrence_ids {
            if !local_ids.insert(local_id.0)
                || !product.local_occurrences.contains_key(&LocalOccurrenceKey {
                    definition_id: definition.id,
                    local_id: *local_id,
                })
            {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
    }
    for feature in product.features.values() {
        ensure_product_id(feature.id.0)?;
        ensure_name(&feature.name)?;
        validate_feature_kind(&feature.kind)?;
        let definition = product
            .definitions
            .get(&feature.definition_id)
            .ok_or(CanonicalError::DefinitionNotFound(feature.definition_id))?;
        if !definition.feature_ids.contains(&feature.id) {
            return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
        }
        match feature.kind.clone() {
            FeatureKind::Extrusion { profile, .. } => {
                let profile = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                if profile.definition_id != feature.definition_id
                    || !matches!(
                        profile.kind,
                        FeatureKind::Profile { .. }
                            | FeatureKind::SegmentProfile { closed: true, .. }
                    )
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Pad(spec) => {
                let sketch = product
                    .features
                    .get(&spec.sketch)
                    .ok_or(CanonicalError::FeatureNotFound(spec.sketch))?;
                let FeatureKind::Sketch(sketch_spec) = &sketch.kind else {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                };
                let workplane = product
                    .features
                    .get(&sketch_spec.workplane)
                    .ok_or(CanonicalError::FeatureNotFound(sketch_spec.workplane))?;
                let region_exists = sketch_spec
                    .solved_regions()
                    .map_err(CanonicalError::from)?
                    .iter()
                    .any(|region| region.id == spec.region);
                if sketch.definition_id != feature.definition_id
                    || workplane.definition_id != feature.definition_id
                    || !matches!(workplane.kind, FeatureKind::Workplane(_))
                    || !region_exists
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::SketchPocket(spec) => {
                let target = product
                    .features
                    .get(&spec.target)
                    .ok_or(CanonicalError::FeatureNotFound(spec.target))?;
                let sketch = product
                    .features
                    .get(&spec.sketch)
                    .ok_or(CanonicalError::FeatureNotFound(spec.sketch))?;
                let FeatureKind::Sketch(sketch_spec) = &sketch.kind else {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                };
                let workplane = product
                    .features
                    .get(&sketch_spec.workplane)
                    .ok_or(CanonicalError::FeatureNotFound(sketch_spec.workplane))?;
                let support_matches = matches!(
                    &workplane.kind,
                    FeatureKind::Workplane(WorkplaneSpec {
                        support: WorkplaneSupport::PlanarFace { reference, health },
                        ..
                    }) if reference.lineage_digest == spec.support.lineage_digest
                        && (*health != WorkplaneSupportHealth::Resolved
                            || reference.as_ref() == spec.support.as_ref())
                );
                let region_exists = sketch_spec
                    .solved_regions()
                    .map_err(CanonicalError::from)?
                    .iter()
                    .any(|region| region.id == spec.region);
                if target.definition_id != feature.definition_id
                    || sketch.definition_id != feature.definition_id
                    || workplane.definition_id != feature.definition_id
                    || !matches!(target.kind, FeatureKind::Pad(_))
                    || spec.support.producer_feature_id != spec.target
                    || !support_matches
                    || !region_exists
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::BottleProfileControl {
                profile,
                body_radius,
                body_height,
                shoulder_rise,
            } => {
                let source = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                if source.definition_id != feature.definition_id
                    || !matches!(source.kind, FeatureKind::Profile { .. })
                    || controlled_bottle_profile(
                        match &source.kind {
                            FeatureKind::Profile { points_mm } => points_mm,
                            _ => unreachable!(),
                        },
                        body_radius.millimetres(),
                        body_height.millimetres(),
                        shoulder_rise.millimetres(),
                    )
                    .is_none()
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Revolve { profile, .. } => {
                let profile_feature = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                if profile_feature.definition_id != feature.definition_id
                    || !matches!(
                        profile_feature.kind,
                        FeatureKind::Profile { .. }
                            | FeatureKind::SegmentProfile { closed: true, .. }
                            | FeatureKind::BottleProfileControl { .. }
                    )
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
            } => {
                let target_feature = product
                    .features
                    .get(&target)
                    .ok_or(CanonicalError::FeatureNotFound(target))?;
                if target == feature.id
                    || target_feature.definition_id != feature.definition_id
                    || !feature_kind_is_solid(&target_feature.kind)
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
                if removed_faces.len() == 1
                    && removed_faces[0].as_str() == BOTTLE_SHELL_OPENING_FACE_ROLE
                {
                    let FeatureKind::Revolve { profile, .. } = target_feature.kind else {
                        return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                    };
                    let profile = product
                        .features
                        .get(&profile)
                        .ok_or(CanonicalError::FeatureNotFound(profile))?;
                    if profile.definition_id != feature.definition_id
                        || !resolved_bottle_profile(product, profile.id).is_some_and(|points_mm| {
                            shell_thickness_is_conservative(&points_mm, thickness.millimetres())
                        })
                    {
                        return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                    }
                }
            }
            FeatureKind::BottleEdgeFinish {
                target,
                edges,
                amount,
                ..
            } => {
                let target_feature = product
                    .features
                    .get(&target)
                    .ok_or(CanonicalError::FeatureNotFound(target))?;
                if target == feature.id
                    || target_feature.definition_id != feature.definition_id
                    || !feature_kind_is_solid(&target_feature.kind)
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
                if edges.len() == 1 && edges[0].as_str() == BOTTLE_SHOULDER_EDGE_ROLE {
                    let FeatureKind::Shell {
                        target: revolve_id, ..
                    } = target_feature.kind
                    else {
                        return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                    };
                    let revolve = product
                        .features
                        .get(&revolve_id)
                        .ok_or(CanonicalError::FeatureNotFound(revolve_id))?;
                    let FeatureKind::Revolve { profile, .. } = revolve.kind else {
                        return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                    };
                    let valid_amount =
                        resolved_bottle_profile(product, profile).is_some_and(|points| {
                            let shoulder_length =
                                (points[3][0] - points[2][0]).hypot(points[3][1] - points[2][1]);
                            amount.millimetres() < shoulder_length * 0.25
                                && amount.millimetres() < points[3][0] * 0.25
                        });
                    if revolve.definition_id != feature.definition_id || !valid_amount {
                        return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                    }
                }
            }
            FeatureKind::TopologyShell {
                target,
                removed_faces,
                ..
            } => {
                validate_topological_feature_context(
                    product.document_id,
                    feature.definition_id,
                    &feature.kind,
                )?;
                validate_topological_target(
                    product,
                    definition,
                    feature.id,
                    target,
                    &removed_faces,
                )?;
            }
            FeatureKind::TopologyEdgeFinish { target, edges, .. } => {
                validate_topological_feature_context(
                    product.document_id,
                    feature.definition_id,
                    &feature.kind,
                )?;
                validate_topological_target(product, definition, feature.id, target, &edges)?;
            }
            FeatureKind::TopologyFaceOffset { target, face, .. } => {
                validate_topological_feature_context(
                    product.document_id,
                    feature.definition_id,
                    &feature.kind,
                )?;
                validate_topological_target(
                    product,
                    definition,
                    feature.id,
                    target,
                    std::slice::from_ref(&face),
                )?;
            }
            FeatureKind::ThroughCut { target, profile } => {
                let target = product
                    .features
                    .get(&target)
                    .ok_or(CanonicalError::FeatureNotFound(target))?;
                let profile = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                if target.definition_id != feature.definition_id
                    || profile.definition_id != feature.definition_id
                    || !matches!(target.kind, FeatureKind::Extrusion { .. })
                    || !matches!(
                        profile.kind,
                        FeatureKind::Profile { .. }
                            | FeatureKind::SegmentProfile { closed: true, .. }
                    )
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Pocket {
                target,
                profile,
                depth,
            } => {
                let target = product
                    .features
                    .get(&target)
                    .ok_or(CanonicalError::FeatureNotFound(target))?;
                let profile = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                let valid_depth = matches!(
                    &target.kind,
                    FeatureKind::Extrusion { height, .. }
                        if depth.millimetres() < height.millimetres()
                );
                if target.definition_id != feature.definition_id
                    || profile.definition_id != feature.definition_id
                    || !valid_depth
                    || !matches!(
                        profile.kind,
                        FeatureKind::Profile { .. }
                            | FeatureKind::SegmentProfile { closed: true, .. }
                    )
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::PlanarOffset { profile, distance } => {
                let source = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                let FeatureKind::Profile { points_mm } = &source.kind else {
                    return Err(CanonicalError::InvalidPlanarOffset);
                };
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let source_precedes_offset = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == profile)
                    .is_some_and(|position| position < feature_position);
                let distance = distance.millimetres();
                let valid_bounds = is_axis_aligned_rectangle(points_mm)
                    && [
                        points_mm[0][0] - distance,
                        points_mm[0][1] - distance,
                        points_mm[2][0] + distance,
                        points_mm[2][1] + distance,
                    ]
                    .into_iter()
                    .all(|coordinate| {
                        coordinate.is_finite() && coordinate.abs() <= MAX_CANONICAL_ABS_MM
                    })
                    && points_mm[1][0] - points_mm[0][0] + 2.0 * distance > PROFILE_EPSILON_MM
                    && points_mm[3][1] - points_mm[0][1] + 2.0 * distance > PROFILE_EPSILON_MM;
                if source.definition_id != feature.definition_id
                    || !source_precedes_offset
                    || !valid_bounds
                {
                    return Err(CanonicalError::InvalidPlanarOffset);
                }
            }
            FeatureKind::Sweep { profile, path } => {
                let profile_source = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                let path_source = product
                    .features
                    .get(&path)
                    .ok_or(CanonicalError::FeatureNotFound(path))?;
                let valid_profile =
                    matches!(
                        &profile_source.kind,
                        FeatureKind::Profile { points_mm } if points_mm.len() >= 3
                    ) || matches!(
                        &profile_source.kind,
                        FeatureKind::SegmentProfile {
                            segments,
                            closed: true,
                        } if segments.len() >= 2
                    ) || matches!(profile_source.kind, FeatureKind::SplineProfile { .. });
                let valid_path = matches!(
                    &path_source.kind,
                    FeatureKind::SegmentProfile {
                        segments,
                        closed: false,
                    } if matches!(segments.as_slice(), [ProfileSegment::Line { .. }])
                );
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let sources_precede_sweep = [profile, path].into_iter().all(|source_id| {
                    definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == source_id)
                        .is_some_and(|position| position < feature_position)
                });
                if profile_source.definition_id != feature.definition_id
                    || path_source.definition_id != feature.definition_id
                    || !valid_profile
                    || !valid_path
                    || !sources_precede_sweep
                {
                    return Err(CanonicalError::InvalidSweep);
                }
            }
            FeatureKind::Loft { sections } => {
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                for section in sections {
                    let profile = product
                        .features
                        .get(&section.profile)
                        .ok_or(CanonicalError::FeatureNotFound(section.profile))?;
                    let source_precedes_loft = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == section.profile)
                        .is_some_and(|position| position < feature_position);
                    if profile.definition_id != feature.definition_id
                        || !matches!(profile.kind, FeatureKind::SplineProfile { .. })
                        || !source_precedes_loft
                    {
                        return Err(CanonicalError::InvalidLoft);
                    }
                }
            }
            FeatureKind::Boolean { target, tool, .. } => {
                let target_feature = product
                    .features
                    .get(&target)
                    .ok_or(CanonicalError::FeatureNotFound(target))?;
                let tool_feature = product
                    .features
                    .get(&tool)
                    .ok_or(CanonicalError::FeatureNotFound(tool))?;
                let produces_body = FeatureKind::produces_body;
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let inputs_precede_boolean = [target, tool].into_iter().all(|input| {
                    definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == input)
                        .is_some_and(|position| position < feature_position)
                });
                if target == tool
                    || target_feature.definition_id != feature.definition_id
                    || tool_feature.definition_id != feature.definition_id
                    || !produces_body(&target_feature.kind)
                    || !produces_body(&tool_feature.kind)
                    || !inputs_precede_boolean
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::ImportedExactBody(spec) => {
                let receipt_is_invalid =
                    product
                        .import_receipts
                        .get(&spec.import_id)
                        .is_none_or(|receipt| {
                            receipt.format() != ImportFormat::Step
                                || receipt.source_sha256() != &spec.source_sha256
                                || receipt.source_byte_len() != spec.source_byte_len
                        });
                if definition.feature_ids.first() != Some(&feature.id) || receipt_is_invalid {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::MeshBody(spec) => {
                let authority_is_invalid = match &spec.authority {
                    MeshAuthority::ExactConversion(conversion) => {
                        conversion.destination_definition_id != feature.definition_id
                            || conversion.destination_feature_id != feature.id
                    }
                    MeshAuthority::ImportedStl { import_id } => product
                        .import_receipts
                        .get(import_id)
                        .is_none_or(|receipt| receipt.format() != ImportFormat::Stl),
                    MeshAuthority::ImportedSketchupScene { import_id } => product
                        .import_receipts
                        .get(import_id)
                        .is_none_or(|receipt| receipt.format() != ImportFormat::SketchupScene),
                    MeshAuthority::Authored { .. } => false,
                };
                if definition.feature_ids.as_slice() != [feature.id] || authority_is_invalid {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Workplane(spec) => match &spec.support {
                WorkplaneSupport::Principal(_) => {}
                WorkplaneSupport::PlanarFace { reference, health } => {
                    let producer = product.features.get(&reference.producer_feature_id).ok_or(
                        CanonicalError::Sketch(SketchError::InvalidPlanarFaceSupport),
                    )?;
                    let profile = product.features.get(&reference.profile_feature_id).ok_or(
                        CanonicalError::Sketch(SketchError::InvalidPlanarFaceSupport),
                    )?;
                    let feature_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == feature.id)
                        .expect("validated definition contains feature");
                    let producer_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == reference.producer_feature_id);
                    let profile_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == reference.profile_feature_id);
                    let evidence = product
                        .exact_reference_evidence
                        .get(&reference.lineage_digest);
                    let evidence_and_frame_are_valid = match health {
                        WorkplaneSupportHealth::Resolved => {
                            evidence.is_some_and(|evidence| evidence.as_ref() == reference.as_ref())
                                && supported_planar_face_frame(product, reference)
                                    == Some(spec.frame)
                        }
                        WorkplaneSupportHealth::Ambiguous
                        | WorkplaneSupportHealth::Lost
                        | WorkplaneSupportHealth::Stale => evidence.is_none(),
                    };
                    if !evidence_and_frame_are_valid
                        || reference.document_id != product.document_id
                        || reference.definition_id != feature.definition_id
                        || producer.definition_id != feature.definition_id
                        || profile.definition_id != feature.definition_id
                        || !feature_kind_is_solid(&producer.kind)
                        || producer_position.is_none_or(|position| position >= feature_position)
                        || profile_position.is_none_or(|position| {
                            producer_position.is_none_or(|producer| position >= producer)
                        })
                    {
                        return Err(CanonicalError::Sketch(
                            SketchError::InvalidPlanarFaceSupport,
                        ));
                    }
                }
                WorkplaneSupport::Offset { base, distance } => {
                    let base_feature = product.features.get(base).ok_or(CanonicalError::Sketch(
                        SketchError::MissingWorkplaneSupport(*base),
                    ))?;
                    let FeatureKind::Workplane(base_spec) = &base_feature.kind else {
                        return Err(CanonicalError::Sketch(
                            SketchError::MissingWorkplaneSupport(*base),
                        ));
                    };
                    let feature_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == feature.id)
                        .expect("validated definition contains feature");
                    let base_precedes = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == *base)
                        .is_some_and(|position| position < feature_position);
                    if base_feature.definition_id != feature.definition_id
                        || !base_precedes
                        || spec.frame != base_spec.frame.offset(distance.millimetres())
                    {
                        return Err(CanonicalError::Sketch(SketchError::WorkplaneCycle(
                            feature.id,
                        )));
                    }
                }
            },
            FeatureKind::Sketch(spec) => {
                let workplane =
                    product
                        .features
                        .get(&spec.workplane)
                        .ok_or(CanonicalError::Sketch(
                            SketchError::MissingWorkplaneSupport(spec.workplane),
                        ))?;
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let workplane_precedes = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == spec.workplane)
                    .is_some_and(|position| position < feature_position);
                if workplane.definition_id != feature.definition_id
                    || !matches!(workplane.kind, FeatureKind::Workplane(_))
                    || !workplane_precedes
                {
                    return Err(CanonicalError::Sketch(
                        SketchError::MissingWorkplaneSupport(spec.workplane),
                    ));
                }
            }
            FeatureKind::Profile { .. }
            | FeatureKind::SegmentProfile { .. }
            | FeatureKind::SplineProfile { .. } => {}
        }
    }
    for (target, binding) in &product.feature_parameter_bindings {
        let feature = product
            .features
            .get(&target.feature_id)
            .ok_or(CanonicalError::FeatureNotFound(target.feature_id))?;
        if binding.target != *target
            || !feature_supports_parameter_target(&feature.kind, target)
            || feature_parameter_value_bits(product, target).is_none()
            || resolve_derived_identity(&product.evaluator_nodes, &binding.derived_from)
                != SlotResolution::Resolved
        {
            return Err(CanonicalError::InvalidFeatureParameterBinding(
                target.clone(),
            ));
        }
    }
    for (target, provenance) in &product.feature_parameter_provenance {
        if !product.feature_parameter_bindings.contains_key(target)
            || provenance.input_digest.is_empty()
            || provenance.result_digest.is_empty()
            || provenance.identity.validate().is_err()
        {
            return Err(CanonicalError::InvalidFeatureParameterBinding(
                target.clone(),
            ));
        }
    }
    for occurrence in product.occurrences.values() {
        ensure_product_id(occurrence.id.0)?;
        ensure_name(&occurrence.name)?;
        validate_transform(occurrence.transform)?;
        if !product.definitions.contains_key(&occurrence.definition_id) {
            return Err(CanonicalError::DefinitionNotFound(occurrence.definition_id));
        }
        if let Some(parent) = occurrence.parent
            && !product.groups.contains_key(&parent)
        {
            return Err(CanonicalError::GroupNotFound(parent));
        }
        if let Some(tag) = occurrence.tag
            && !product.tags.contains_key(&tag)
        {
            return Err(CanonicalError::TagNotFound(tag));
        }
    }
    for group in product.groups.values() {
        ensure_product_id(group.id.0)?;
        ensure_name(&group.name)?;
        validate_transform(group.transform)?;
        if let Some(parent) = group.parent
            && !product.groups.contains_key(&parent)
        {
            return Err(CanonicalError::GroupNotFound(parent));
        }
        let mut visiting = BTreeSet::new();
        let mut cursor = Some(group.id);
        while let Some(group_id) = cursor {
            if !visiting.insert(group_id) {
                return Err(CanonicalError::GroupCycle(group_id));
            }
            cursor = product.groups[&group_id].parent;
        }
    }
    for (key, group) in &product.local_groups {
        if key != &group.key
            || !product.definitions.contains_key(&key.definition_id)
            || !product.definitions[&key.definition_id]
                .local_group_ids
                .contains(&key.local_id)
        {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        if let Some(parent) = group.parent {
            let parent_key = LocalGroupKey {
                definition_id: key.definition_id,
                local_id: parent,
            };
            if !product.local_groups.contains_key(&parent_key) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
        let mut visiting = BTreeSet::new();
        let mut cursor = Some(key.local_id);
        while let Some(local_id) = cursor {
            if !visiting.insert(local_id) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
            cursor = product.local_groups[&LocalGroupKey {
                definition_id: key.definition_id,
                local_id,
            }]
                .parent;
        }
    }
    for (key, occurrence) in &product.local_occurrences {
        if key != &occurrence.key
            || !product.definitions.contains_key(&key.definition_id)
            || !product.definitions[&key.definition_id]
                .local_occurrence_ids
                .contains(&key.local_id)
            || !product.definitions.contains_key(&occurrence.definition_id)
        {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        if let Some(parent) = occurrence.parent {
            let parent_key = LocalGroupKey {
                definition_id: key.definition_id,
                local_id: parent,
            };
            if !product.local_groups.contains_key(&parent_key) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
        if let Some(tag) = occurrence.tag
            && !product.tags.contains_key(&tag)
        {
            return Err(CanonicalError::TagNotFound(tag));
        }
    }
    let feature_graph = FeatureDependencyGraph::from_product(product)?;
    for ((definition_id, body_id), suppressed) in &product.body_feature_suppression {
        if suppressed.is_empty() {
            return Err(CanonicalError::InvalidFeatureSuppression(
                *definition_id,
                *body_id,
            ));
        }
        let ordered =
            ordered_body_feature_history(product, *definition_id, *body_id, &feature_graph)?;
        let ordered_suppressed = ordered
            .into_iter()
            .filter(|id| suppressed.contains(id))
            .collect::<Vec<_>>();
        if ordered_suppressed.len() != suppressed.len() {
            return Err(CanonicalError::InvalidFeatureSuppression(
                *definition_id,
                *body_id,
            ));
        }
        validate_body_feature_suppression(
            product,
            *definition_id,
            *body_id,
            &ordered_suppressed,
            &feature_graph,
        )?;
    }
    validate_definition_ownership_graph(product)
}

fn validate_definition_ownership_graph(product: &ProductModel) -> Result<(), CanonicalError> {
    fn visit(
        definition_id: DefinitionId,
        product: &ProductModel,
        visiting: &mut BTreeSet<DefinitionId>,
        visited: &mut BTreeSet<DefinitionId>,
    ) -> Result<(), CanonicalError> {
        if visited.contains(&definition_id) {
            return Ok(());
        }
        if !visiting.insert(definition_id) {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        let definition = &product.definitions[&definition_id];
        for local_id in &definition.local_occurrence_ids {
            let target = product.local_occurrences[&LocalOccurrenceKey {
                definition_id,
                local_id: *local_id,
            }]
                .definition_id;
            visit(target, product, visiting, visited)?;
        }
        visiting.remove(&definition_id);
        visited.insert(definition_id);
        Ok(())
    }

    let mut visited = BTreeSet::new();
    for definition_id in product.definitions.keys().cloned() {
        visit(definition_id, product, &mut BTreeSet::new(), &mut visited)?;
    }
    Ok(())
}

fn validate_high_risk_requester(
    principal: ProposalPrincipal,
) -> Result<(), HumanConfirmationError> {
    if matches!(
        principal,
        ProposalPrincipal::ManualClient
            | ProposalPrincipal::Human(0)
            | ProposalPrincipal::Plugin(0)
    ) {
        return Err(HumanConfirmationError::UnidentifiedRequester);
    }
    Ok(())
}

fn authenticated_human(approver: AuthenticatedApprover) -> Result<u64, HumanConfirmationError> {
    match approver {
        AuthenticatedApprover::Human(id) if id != 0 => Ok(id),
        AuthenticatedApprover::Human(_) => Err(HumanConfirmationError::InvalidHumanPrincipal),
        AuthenticatedApprover::Machine(_) => Err(HumanConfirmationError::MachineCannotApprove),
    }
}

fn validate_confirmation_lifetime(
    issued_at_ms: u64,
    expires_at_ms: u64,
) -> Result<(), HumanConfirmationError> {
    if expires_at_ms <= issued_at_ms
        || expires_at_ms - issued_at_ms > MAX_HUMAN_CONFIRMATION_LIFETIME_MS
    {
        return Err(HumanConfirmationError::InvalidLifetime);
    }
    Ok(())
}

fn validate_confirmation_requirement(
    context: &ProposalContext,
) -> Result<(), ProposalPrepareError> {
    if matches!(context.risk, ProposalRisk::High(_))
        && matches!(
            context.principal,
            ProposalPrincipal::ManualClient
                | ProposalPrincipal::Human(0)
                | ProposalPrincipal::Plugin(0)
        )
    {
        return Err(ProposalPrepareError::Confirmation(
            HumanConfirmationError::UnidentifiedRequester,
        ));
    }
    match (context.risk, &context.confirmation) {
        (ProposalRisk::Standard, ProposalConfirmation::ReviewRequired) => Ok(()),
        (ProposalRisk::High(class), ProposalConfirmation::HumanOnly(scope))
            if class == scope.class =>
        {
            Ok(())
        }
        _ => Err(ProposalPrepareError::Confirmation(
            HumanConfirmationError::ConfirmationRequirementMismatch,
        )),
    }
}

fn push_confirmation_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn push_confirmation_field(bytes: &mut Vec<u8>, value: &[u8]) {
    push_confirmation_u64(bytes, u64::try_from(value.len()).unwrap_or(u64::MAX));
    bytes.extend_from_slice(value);
}

fn push_confirmation_optional_field(bytes: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            bytes.push(1);
            push_confirmation_field(bytes, value.as_bytes());
        }
        None => bytes.push(0),
    }
}

fn push_confirmation_principal(bytes: &mut Vec<u8>, principal: ProposalPrincipal) {
    match principal {
        ProposalPrincipal::ManualClient => bytes.push(0),
        ProposalPrincipal::Human(id) => {
            bytes.push(1);
            push_confirmation_u64(bytes, id);
        }
        ProposalPrincipal::LocalAssistant => bytes.push(2),
        ProposalPrincipal::Plugin(id) => {
            bytes.push(3);
            push_confirmation_u64(bytes, id);
        }
    }
}

fn push_confirmation_scope(bytes: &mut Vec<u8>, scope: &HighRiskScope) {
    bytes.push(match scope.class {
        HighRiskClass::DestructiveBulkChange => 0,
        HighRiskClass::Overwrite => 1,
        HighRiskClass::LossyConversion => 2,
        HighRiskClass::ExternalDisclosure => 3,
        HighRiskClass::ReleaseManufacturingExportWithWarnings => 4,
        HighRiskClass::CapabilityExpansion => 5,
    });
    push_confirmation_optional_field(bytes, scope.destination());
    push_confirmation_optional_field(bytes, scope.provider());
    push_confirmation_optional_field(bytes, scope.path());
}

fn validate_proposal_budget(
    requested: ProposalBudget,
    cost: ProposalCost,
) -> Result<(), ProposalPrepareError> {
    if requested.max_commands > ProposalBudget::HOST_MAX.max_commands
        || requested.max_read_dependencies > ProposalBudget::HOST_MAX.max_read_dependencies
        || requested.max_write_targets > ProposalBudget::HOST_MAX.max_write_targets
    {
        return Err(ProposalPrepareError::HostBudgetExceeded);
    }
    if cost.commands > requested.max_commands
        || cost.read_dependencies > requested.max_read_dependencies
        || cost.write_targets > requested.max_write_targets
    {
        return Err(ProposalPrepareError::RequestedBudgetExceeded);
    }
    Ok(())
}

fn proposal_candidate(
    snapshot: &Snapshot,
    batch: &CommandBatch,
    writes: &BTreeSet<AuthoritativeDependency>,
    goal: ProposalGoal,
) -> Result<(Vec<ProposalDiffEntry>, String), ProposalPrepareError> {
    let mut candidate =
        DocumentStore::from_product(snapshot.revision_id, snapshot.product.as_ref().clone())?;
    let revision = candidate.apply_batch(batch)?;
    let after = revision.snapshot();
    let diff = writes
        .iter()
        .cloned()
        .map(|target| ProposalDiffEntry {
            target: target.clone(),
            before: proposal_value(snapshot, target.clone(), goal.clone()),
            after: proposal_value(after, target, goal.clone()),
        })
        .collect();
    Ok((diff, dependency_digest(after, writes)))
}

fn tip_replacement_candidate(
    parent: &Snapshot,
    corrected_revision: u64,
    batch: &CommandBatch,
    writes: &BTreeSet<AuthoritativeDependency>,
    goal: ProposalGoal,
) -> Result<(Snapshot, Vec<ProposalDiffEntry>, String), ProposalPrepareError> {
    let mut candidate =
        DocumentStore::from_product(parent.revision_id, parent.product.as_ref().clone())?;
    candidate.next_revision_id = corrected_revision;
    let revision = candidate.apply_batch(batch)?;
    let after = revision.snapshot();
    let diff = writes
        .iter()
        .cloned()
        .map(|target| ProposalDiffEntry {
            target: target.clone(),
            before: proposal_value(parent, target.clone(), goal.clone()),
            after: proposal_value(after, target, goal.clone()),
        })
        .collect();
    Ok((after.clone(), diff, dependency_digest(after, writes)))
}

fn proposal_value(
    snapshot: &Snapshot,
    target: AuthoritativeDependency,
    goal: ProposalGoal,
) -> ProposalValue {
    match target {
        AuthoritativeDependency::EvaluatorNode(id) => {
            snapshot
                .evaluator_node(id)
                .map_or(ProposalValue::Missing, |node| {
                    if matches!(goal, ProposalGoal::CreateEvaluatorInput(_)) {
                        node.dimension()
                            .map_or(ProposalValue::Missing, |dimension| {
                                ProposalValue::EvaluatorInputState {
                                    name: node.name().to_owned(),
                                    dimension: dimension.clone(),
                                    dependencies: node.dependencies().to_vec(),
                                }
                            })
                    } else if matches!(goal, ProposalGoal::CreateEvaluatorExpression(_)) {
                        ProposalValue::EvaluatorExpressionState {
                            name: node.name().to_owned(),
                            expression: node.kind().source().to_owned(),
                            dependencies: node.dependencies().to_vec(),
                        }
                    } else if matches!(goal, ProposalGoal::CreateEvaluatorRule(_)) {
                        match node.kind() {
                            EvaluatorNodeKind::Rule {
                                outputs,
                                allowed_parameters,
                                ..
                            } => ProposalValue::EvaluatorRuleState {
                                name: node.name().to_owned(),
                                expression: node.kind().source().to_owned(),
                                dependencies: node.dependencies().to_vec(),
                                input_ports: node.input_ports().to_vec(),
                                output_ports: node.output_ports().to_vec(),
                                outputs: outputs.clone(),
                                override_parameters: allowed_parameters.clone(),
                            },
                            _ => ProposalValue::Missing,
                        }
                    } else if matches!(goal, ProposalGoal::RenameEvaluatorNode(_)) {
                        ProposalValue::Text(node.name().to_owned())
                    } else if matches!(goal, ProposalGoal::SetEvaluatorExpression(_)) {
                        ProposalValue::Text(node.kind().source().to_owned())
                    } else if matches!(goal, ProposalGoal::SetRuleOutputs(_)) {
                        match node.kind() {
                            EvaluatorNodeKind::Rule { outputs, .. } => {
                                ProposalValue::RuleOutputs(outputs.clone())
                            }
                            _ => ProposalValue::Missing,
                        }
                    } else {
                        node.dimension()
                            .cloned()
                            .map_or(ProposalValue::Missing, ProposalValue::Dimension)
                    }
                })
        }
        AuthoritativeDependency::Override(id)
            if matches!(
                goal,
                ProposalGoal::CreateRuleOverride(_) | ProposalGoal::DeleteRuleOverride(_)
            ) =>
        {
            snapshot
                .override_by_id(id)
                .map_or(ProposalValue::Missing, |value| {
                    ProposalValue::RuleOverrideState {
                        target: value.target.clone(),
                        parameter: value.parameter.clone(),
                        value: value.value(),
                        health: value.health.clone(),
                    }
                })
        }
        AuthoritativeDependency::FeatureParameterBinding(target)
            if matches!(
                goal,
                ProposalGoal::CreateFeatureParameterBinding(_)
                    | ProposalGoal::DeleteFeatureParameterBinding(_)
            ) =>
        {
            snapshot
                .feature_parameter_binding(&target)
                .map_or(ProposalValue::Missing, |binding| {
                    ProposalValue::FeatureParameterBindingState {
                        target: binding.target.clone(),
                        derived_from: binding.derived_from.clone(),
                    }
                })
        }
        AuthoritativeDependency::Joint(id)
            if matches!(
                goal,
                ProposalGoal::CreateJoint(_) | ProposalGoal::DeleteJoint(_)
            ) =>
        {
            snapshot
                .joint(id)
                .map_or(ProposalValue::Missing, |joint| ProposalValue::JointState {
                    participant_a: joint.participant_a().clone(),
                    participant_b: joint.participant_b().clone(),
                    volume_min: joint.volume().min(),
                    volume_max: joint.volume().max(),
                })
        }
        AuthoritativeDependency::Space(id)
            if matches!(
                goal,
                ProposalGoal::CreateSpace(_) | ProposalGoal::DeleteSpace(_)
            ) =>
        {
            snapshot
                .space(id)
                .map_or(ProposalValue::Missing, |space| ProposalValue::SpaceState {
                    purpose: space.purpose().to_owned(),
                    volume_min: space.volume().min(),
                    volume_max: space.volume().max(),
                    adjacent_to: space.adjacent_to().to_vec(),
                    accessible_to: space.accessible_to().to_vec(),
                })
        }
        AuthoritativeDependency::ClearanceVolume(id)
            if matches!(
                goal,
                ProposalGoal::CreateClearanceVolume(_) | ProposalGoal::DeleteClearanceVolume(_)
            ) =>
        {
            snapshot
                .clearance_volume(id)
                .map_or(ProposalValue::Missing, |clearance| {
                    ProposalValue::ClearanceVolumeState {
                        owner: clearance.owner().clone(),
                        reason: clearance.reason().to_owned(),
                        volume_min: clearance.volume().min(),
                        volume_max: clearance.volume().max(),
                        coordinate_frame: clearance.coordinate_frame(),
                        tolerance_mm: clearance.tolerance().epsilon_mm(),
                        severity: clearance.severity(),
                        derived_from: clearance.derived_from().cloned(),
                    }
                })
        }
        AuthoritativeDependency::PersistentDimension(id)
            if matches!(
                goal,
                ProposalGoal::CreatePersistentDimension(_)
                    | ProposalGoal::DeletePersistentDimension(_)
            ) =>
        {
            snapshot
                .persistent_dimension(id)
                .map_or(ProposalValue::Missing, |dimension| {
                    ProposalValue::PersistentDimensionState {
                        name: dimension.name.clone(),
                        target: dimension.target.clone(),
                        presentation: dimension.presentation,
                    }
                })
        }
        AuthoritativeDependency::Feature(id) => {
            snapshot
                .feature(id)
                .map_or(ProposalValue::Missing, |feature| {
                    if let ProposalGoal::RecomputeFeatureParameter(ref parameter) = goal
                        && parameter.feature_id == id
                    {
                        return feature_parameter_dimension(&snapshot.product, parameter)
                            .map_or(ProposalValue::Missing, ProposalValue::Dimension);
                    }
                    match feature.kind() {
                        FeatureKind::BottleProfileControl {
                            body_radius,
                            body_height,
                            shoulder_rise,
                            ..
                        } => match goal {
                            ProposalGoal::SetBottleControlDimension(
                                _,
                                BottleControlDimension::BodyRadius,
                            ) => ProposalValue::Dimension(body_radius.clone()),
                            ProposalGoal::SetBottleControlDimension(
                                _,
                                BottleControlDimension::BodyHeight,
                            ) => ProposalValue::Dimension(body_height.clone()),
                            ProposalGoal::SetBottleControlDimension(
                                _,
                                BottleControlDimension::ShoulderRise,
                            ) => ProposalValue::Dimension(shoulder_rise.clone()),
                            _ => ProposalValue::Digest(dependency_digest(
                                snapshot,
                                &BTreeSet::from([target]),
                            )),
                        },
                        FeatureKind::BottleEdgeFinish { kind, .. }
                            if matches!(goal, ProposalGoal::SetBottleEdgeFinishKind(_)) =>
                        {
                            ProposalValue::BottleEdgeFinishKind(*kind)
                        }
                        FeatureKind::Profile { points_mm }
                            if matches!(goal, ProposalGoal::SetProfilePoints(_)) =>
                        {
                            ProposalValue::ProfilePoints(points_mm.clone())
                        }
                        FeatureKind::Profile { points_mm }
                            if matches!(
                                goal,
                                ProposalGoal::CreateProfileFeature(_)
                                    | ProposalGoal::DeleteProfileFeature(_)
                                    | ProposalGoal::CloneProfileDefinitionAndRepoint(_)
                            ) =>
                        {
                            ProposalValue::ProfileFeatureState {
                                definition: feature.definition_id(),
                                name: feature.name().to_owned(),
                                points_mm: points_mm.clone(),
                            }
                        }
                        FeatureKind::Extrusion { height, .. } => {
                            ProposalValue::Dimension(height.clone())
                        }
                        FeatureKind::Shell { thickness, .. } => {
                            ProposalValue::Dimension(thickness.clone())
                        }
                        FeatureKind::BottleEdgeFinish { amount, .. } => {
                            ProposalValue::Dimension(amount.clone())
                        }
                        _ => ProposalValue::Digest(dependency_digest(
                            snapshot,
                            &BTreeSet::from([target]),
                        )),
                    }
                })
        }
        AuthoritativeDependency::Tag(id) => {
            snapshot
                .tag(id)
                .map_or(ProposalValue::Missing, |tag| match goal {
                    ProposalGoal::SetTagVisibility(_) => ProposalValue::Boolean(tag.visible()),
                    ProposalGoal::CreateTag(_) | ProposalGoal::DeleteTag(_) => {
                        ProposalValue::TagState {
                            name: tag.name().to_owned(),
                            visible: tag.visible(),
                        }
                    }
                    _ => ProposalValue::Digest(dependency_digest(
                        snapshot,
                        &BTreeSet::from([target]),
                    )),
                })
        }
        AuthoritativeDependency::Collection(id)
            if matches!(
                goal,
                ProposalGoal::SetCollectionOccurrences(_)
                    | ProposalGoal::CreateCollection(_)
                    | ProposalGoal::DeleteCollection(_)
            ) =>
        {
            snapshot
                .collection(id)
                .map_or(ProposalValue::Missing, |collection| match goal {
                    ProposalGoal::CreateCollection(_) => {
                        ProposalValue::Text(collection.name().to_owned())
                    }
                    ProposalGoal::SetCollectionOccurrences(_) => {
                        ProposalValue::Occurrences(collection.occurrence_ids().collect())
                    }
                    ProposalGoal::DeleteCollection(_) => ProposalValue::CollectionState {
                        name: collection.name().to_owned(),
                        occurrence_ids: collection.occurrence_ids().collect(),
                    },
                    _ => unreachable!(),
                })
        }
        AuthoritativeDependency::Definition(id) => {
            snapshot
                .definition(id)
                .map_or(ProposalValue::Missing, |definition| {
                    if matches!(
                        goal,
                        ProposalGoal::RenameDefinition(_) | ProposalGoal::CreateDefinition(_)
                    ) {
                        ProposalValue::Text(definition.name().to_owned())
                    } else if matches!(
                        goal,
                        ProposalGoal::DeleteDefinition(_)
                            | ProposalGoal::CloneProfileDefinitionAndRepoint(_)
                            | ProposalGoal::ConvertEmptyGroupToComponent(_)
                    ) {
                        ProposalValue::DefinitionState {
                            name: definition.name().to_owned(),
                            feature_ids: definition.feature_ids().to_vec(),
                            local_occurrence_ids: definition.local_occurrence_ids().to_vec(),
                            local_group_ids: definition.local_group_ids().to_vec(),
                        }
                    } else if matches!(
                        goal,
                        ProposalGoal::CreateProfileFeature(_)
                            | ProposalGoal::DeleteProfileFeature(_)
                    ) {
                        ProposalValue::DefinitionFeatures(definition.feature_ids().to_vec())
                    } else {
                        ProposalValue::Digest(dependency_digest(
                            snapshot,
                            &BTreeSet::from([target]),
                        ))
                    }
                })
        }
        AuthoritativeDependency::Occurrence(id) => {
            snapshot
                .occurrence(id)
                .map_or(ProposalValue::Missing, |occurrence| match goal {
                    ProposalGoal::SetOccurrenceTranslation(_) => {
                        ProposalValue::Transform(occurrence.transform())
                    }
                    ProposalGoal::AtomicMultiCommandEdit(_) => ProposalValue::OccurrenceState {
                        definition: occurrence.definition_id(),
                        name: occurrence.name().to_owned(),
                        transform: occurrence.transform(),
                        parent: occurrence.parent(),
                        tag: occurrence.tag(),
                        visible: occurrence.visible(),
                    },
                    ProposalGoal::SetOccurrenceTag(_) => ProposalValue::Tag(occurrence.tag()),
                    ProposalGoal::RepointOccurrence(_) => {
                        ProposalValue::Definition(occurrence.definition_id())
                    }
                    ProposalGoal::SetOccurrenceParent(_) => {
                        ProposalValue::Group(occurrence.parent())
                    }
                    ProposalGoal::CreateOccurrence(_)
                    | ProposalGoal::DeleteOccurrence(_)
                    | ProposalGoal::CloneProfileDefinitionAndRepoint(_)
                    | ProposalGoal::ConvertEmptyGroupToComponent(_) => {
                        ProposalValue::OccurrenceState {
                            definition: occurrence.definition_id(),
                            name: occurrence.name().to_owned(),
                            transform: occurrence.transform(),
                            parent: occurrence.parent(),
                            tag: occurrence.tag(),
                            visible: occurrence.visible(),
                        }
                    }
                    _ => ProposalValue::Boolean(occurrence.visible()),
                })
        }
        AuthoritativeDependency::GroupSubtree(id)
            if matches!(goal, ProposalGoal::ConvertEmptyGroupToComponent(_)) =>
        {
            snapshot
                .group(id)
                .map_or(ProposalValue::Missing, |group| ProposalValue::GroupState {
                    name: group.name().to_owned(),
                    transform: group.transform(),
                    parent: group.parent(),
                })
        }
        AuthoritativeDependency::Group(id)
            if matches!(
                goal,
                ProposalGoal::SetGroupTranslation(_)
                    | ProposalGoal::SetGroupParent(_)
                    | ProposalGoal::CreateGroup(_)
                    | ProposalGoal::DeleteGroup(_)
            ) =>
        {
            snapshot
                .group(id)
                .map_or(ProposalValue::Missing, |group| match goal {
                    ProposalGoal::SetGroupTranslation(_) => {
                        ProposalValue::Transform(group.transform())
                    }
                    ProposalGoal::SetGroupParent(_) => ProposalValue::Group(group.parent()),
                    ProposalGoal::CreateGroup(_) | ProposalGoal::DeleteGroup(_) => {
                        ProposalValue::GroupState {
                            name: group.name().to_owned(),
                            transform: group.transform(),
                            parent: group.parent(),
                        }
                    }
                    _ => unreachable!(),
                })
        }
        _ => ProposalValue::Digest(dependency_digest(snapshot, &BTreeSet::from([target]))),
    }
}

fn authoritative_writes(
    snapshot: &Snapshot,
    batch: &CommandBatch,
) -> BTreeSet<AuthoritativeDependency> {
    let mut writes = BTreeSet::new();
    for command in &batch.commands {
        match command {
            CanonicalCommand::CreateEvaluatorNode { id, .. }
            | CanonicalCommand::SetEvaluatorDimension { id, .. }
            | CanonicalCommand::RenameEvaluatorNode { id, .. }
            | CanonicalCommand::CreateExpressionNode { id, .. }
            | CanonicalCommand::CreateRuleNode { id, .. }
            | CanonicalCommand::SetNodeExpression { id, .. }
            | CanonicalCommand::SetRuleOutputs { id, .. } => {
                writes.insert(AuthoritativeDependency::EvaluatorNode(*id));
            }
            CanonicalCommand::UpsertOverride(value) => {
                writes.insert(AuthoritativeDependency::Override(value.id));
            }
            CanonicalCommand::DeleteOverride { id } => {
                writes.insert(AuthoritativeDependency::Override(*id));
            }
            CanonicalCommand::UpsertFeatureParameterBinding(binding) => {
                writes.insert(AuthoritativeDependency::FeatureParameterBinding(
                    binding.target.clone(),
                ));
            }
            CanonicalCommand::DeleteFeatureParameterBinding { target } => {
                writes.insert(AuthoritativeDependency::FeatureParameterBinding(
                    target.clone(),
                ));
            }
            CanonicalCommand::RecomputeFeatureParameters { .. } => {
                writes.extend(
                    snapshot
                        .feature_parameter_bindings()
                        .map(|binding| AuthoritativeDependency::Feature(binding.target.feature_id)),
                );
            }
            CanonicalCommand::UpsertJoint(joint) => {
                writes.insert(AuthoritativeDependency::Joint(joint.id()));
            }
            CanonicalCommand::DeleteJoint { id } => {
                writes.insert(AuthoritativeDependency::Joint(*id));
            }
            CanonicalCommand::UpsertSpace(space) => {
                writes.insert(AuthoritativeDependency::Space(space.id()));
            }
            CanonicalCommand::DeleteSpace { id } => {
                writes.insert(AuthoritativeDependency::Space(*id));
            }
            CanonicalCommand::UpsertClearanceVolume(clearance) => {
                writes.insert(AuthoritativeDependency::ClearanceVolume(clearance.id()));
            }
            CanonicalCommand::DeleteClearanceVolume { id } => {
                writes.insert(AuthoritativeDependency::ClearanceVolume(*id));
            }
            CanonicalCommand::UpsertPersistentDimension(dimension) => {
                writes.insert(AuthoritativeDependency::PersistentDimension(dimension.id));
            }
            CanonicalCommand::DeletePersistentDimension { id } => {
                writes.insert(AuthoritativeDependency::PersistentDimension(*id));
            }
            CanonicalCommand::CreateTag { id, .. }
            | CanonicalCommand::DeleteTag { id }
            | CanonicalCommand::SetTagVisibility { id, .. }
            | CanonicalCommand::SetTagName { id, .. } => {
                writes.insert(AuthoritativeDependency::Tag(*id));
            }
            CanonicalCommand::UpsertClassificationDimension { id, .. } => {
                writes.insert(AuthoritativeDependency::ClassificationDimension(*id));
            }
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id,
                dimension_id,
                ..
            } => {
                writes.insert(AuthoritativeDependency::OccurrenceClassification(
                    *occurrence_id,
                    *dimension_id,
                ));
            }
            CanonicalCommand::CreateCollection { id, .. }
            | CanonicalCommand::DeleteCollection { id }
            | CanonicalCommand::SetCollectionOccurrences { id, .. } => {
                writes.insert(AuthoritativeDependency::Collection(*id));
            }
            CanonicalCommand::RecordImport(receipt) => {
                writes.insert(AuthoritativeDependency::Import(receipt.id()));
            }
            CanonicalCommand::CreateDefinition { id, .. }
            | CanonicalCommand::DeleteDefinition { id }
            | CanonicalCommand::RenameDefinition { id, .. } => {
                writes.insert(AuthoritativeDependency::Definition(*id));
            }
            CanonicalCommand::CreateBody { definition_id, .. }
            | CanonicalCommand::DeleteBody { definition_id, .. }
            | CanonicalCommand::RenameBody { definition_id, .. }
            | CanonicalCommand::SetActiveBody { definition_id, .. }
            | CanonicalCommand::SetBodyVisibility { definition_id, .. }
            | CanonicalCommand::ConsumeBody { definition_id, .. } => {
                writes.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::SetFeatureBodyOwnership { id, .. } => {
                writes.insert(AuthoritativeDependency::Feature(*id));
                if let Some(feature) = snapshot.feature(*id) {
                    writes.insert(AuthoritativeDependency::Definition(feature.definition_id()));
                }
            }
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id,
                body_id,
                ..
            } => {
                writes.insert(AuthoritativeDependency::BodyFeatureSuppression(
                    *definition_id,
                    *body_id,
                ));
            }
            CanonicalCommand::CreateFeature {
                id, definition_id, ..
            } => {
                writes.insert(AuthoritativeDependency::Feature(*id));
                writes.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::DeleteFeature { id } => {
                writes.insert(AuthoritativeDependency::Feature(*id));
                if let Some(feature) = snapshot.feature(*id) {
                    writes.insert(AuthoritativeDependency::Definition(feature.definition_id()));
                }
            }
            CanonicalCommand::SetFeatureDimension { id, .. }
            | CanonicalCommand::SetSketchConstraintDimension { id, .. }
            | CanonicalCommand::TranslateProfile { id, .. }
            | CanonicalCommand::SetBottleControlDimension { id, .. }
            | CanonicalCommand::SetBottleEdgeFinishKind { id, .. }
            | CanonicalCommand::SetProfilePoints { id, .. } => {
                writes.insert(AuthoritativeDependency::Feature(*id));
            }
            CanonicalCommand::GuardAssemblyRecompute { .. } => {}
            CanonicalCommand::ApplyAssemblySolve { transforms, .. } => {
                writes.extend(
                    transforms
                        .iter()
                        .map(|(id, _)| AuthoritativeDependency::Occurrence(*id)),
                );
            }
            CanonicalCommand::SetOccurrenceGrounded { id, .. } => {
                writes.insert(AuthoritativeDependency::GroundedOccurrence(*id));
            }
            CanonicalCommand::CreateAssemblyMate(mate)
            | CanonicalCommand::RebindAssemblyMate(mate) => {
                writes.insert(AuthoritativeDependency::AssemblyMate(mate.id()));
            }
            CanonicalCommand::SetAssemblyMateKind { id, .. }
            | CanonicalCommand::DeleteAssemblyMate { id } => {
                writes.insert(AuthoritativeDependency::AssemblyMate(*id));
            }
            CanonicalCommand::CreateAssemblyJoint(joint) => {
                writes.insert(AuthoritativeDependency::AssemblyJoint(joint.id()));
            }
            CanonicalCommand::SetAssemblyJointKind { id, .. }
            | CanonicalCommand::SetAssemblyJointPosition { id, .. }
            | CanonicalCommand::SetAssemblyJointLimits { id, .. }
            | CanonicalCommand::DeleteAssemblyJoint { id } => {
                writes.insert(AuthoritativeDependency::AssemblyJoint(*id));
            }
            CanonicalCommand::CreateAssemblyMotionCoupling(coupling)
            | CanonicalCommand::UpdateAssemblyMotionCoupling(coupling) => {
                writes.insert(AuthoritativeDependency::AssemblyMotionCoupling(
                    coupling.id(),
                ));
            }
            CanonicalCommand::DeleteAssemblyMotionCoupling { id } => {
                writes.insert(AuthoritativeDependency::AssemblyMotionCoupling(*id));
            }
            CanonicalCommand::CreateAssemblyMotionStudy(study)
            | CanonicalCommand::UpdateAssemblyMotionStudy(study) => {
                writes.insert(AuthoritativeDependency::AssemblyMotionStudy(study.id()));
            }
            CanonicalCommand::DeleteAssemblyMotionStudy { id } => {
                writes.insert(AuthoritativeDependency::AssemblyMotionStudy(*id));
            }
            CanonicalCommand::CreateDrawingSheet(sheet)
            | CanonicalCommand::UpdateDrawingSheet(sheet) => {
                writes.insert(AuthoritativeDependency::DrawingSheet(sheet.id()));
            }
            CanonicalCommand::DeleteDrawingSheet { id } => {
                writes.insert(AuthoritativeDependency::DrawingSheet(*id));
            }
            CanonicalCommand::CreateOccurrence { id, .. }
            | CanonicalCommand::DeleteOccurrence { id }
            | CanonicalCommand::SetOccurrenceTransform { id, .. }
            | CanonicalCommand::RenameEntity { id, .. }
            | CanonicalCommand::SetOccurrenceVisibility { id, .. }
            | CanonicalCommand::SetOccurrenceTag { id, .. }
            | CanonicalCommand::RepointOccurrence { id, .. }
            | CanonicalCommand::SetOccurrenceParent { id, .. } => {
                writes.insert(AuthoritativeDependency::Occurrence(*id));
            }
            CanonicalCommand::CreateGroup { id, .. }
            | CanonicalCommand::DeleteGroup { id }
            | CanonicalCommand::SetGroupTransform { id, .. }
            | CanonicalCommand::SetGroupParent { id, .. } => {
                writes.insert(AuthoritativeDependency::Group(*id));
            }
            CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                writes.insert(AuthoritativeDependency::Occurrence(plan.occurrence_id));
                writes.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                for (_, new_id) in &plan.feature_id_map {
                    writes.insert(AuthoritativeDependency::Feature(*new_id));
                }
            }
            CanonicalCommand::ConvertGroupToComponent(plan) => {
                writes.insert(AuthoritativeDependency::GroupSubtree(plan.group_id));
                writes.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                writes.insert(AuthoritativeDependency::Occurrence(plan.new_occurrence_id));
            }
            CanonicalCommand::ApplySolidTool(plan) => {
                writes.insert(AuthoritativeDependency::Occurrence(
                    plan.target_occurrence_id,
                ));
                writes.insert(AuthoritativeDependency::Occurrence(plan.tool_occurrence_id));
                writes.insert(AuthoritativeDependency::Definition(
                    plan.result_definition_id,
                ));
                writes.extend(
                    plan.result_feature_ids
                        .iter()
                        .cloned()
                        .map(AuthoritativeDependency::Feature),
                );
            }
        }
    }
    writes
}

fn authoritative_dependencies(
    snapshot: &Snapshot,
    batch: &CommandBatch,
) -> BTreeSet<AuthoritativeDependency> {
    let mut dependencies = BTreeSet::new();
    for command in &batch.commands {
        match command {
            CanonicalCommand::CreateEvaluatorNode {
                id,
                dependencies: node_dependencies,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
                for dependency in node_dependencies {
                    add_evaluator_dependency_closure(snapshot, *dependency, &mut dependencies);
                }
            }
            CanonicalCommand::SetEvaluatorDimension { id, .. }
            | CanonicalCommand::RenameEvaluatorNode { id, .. } => {
                add_evaluator_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::RecordImport(receipt) => {
                dependencies.insert(AuthoritativeDependency::Import(receipt.id()));
                for output in receipt.outputs() {
                    dependencies.insert(match output {
                        ImportOutputRef::Definition(id) => AuthoritativeDependency::Definition(*id),
                        ImportOutputRef::Feature(id) => AuthoritativeDependency::Feature(*id),
                        ImportOutputRef::Occurrence(id) => AuthoritativeDependency::Occurrence(*id),
                    });
                }
            }
            CanonicalCommand::CreateDefinition { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
            }
            CanonicalCommand::DeleteDefinition { id } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
                dependencies.insert(AuthoritativeDependency::DefinitionUsers(*id));
            }
            CanonicalCommand::RenameDefinition { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
            }
            CanonicalCommand::CreateBody { definition_id, .. }
            | CanonicalCommand::DeleteBody { definition_id, .. }
            | CanonicalCommand::RenameBody { definition_id, .. }
            | CanonicalCommand::SetActiveBody { definition_id, .. }
            | CanonicalCommand::SetBodyVisibility { definition_id, .. } => {
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::ConsumeBody {
                definition_id,
                by_feature_id,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                add_feature_dependency_closure(snapshot, *by_feature_id, &mut dependencies);
            }
            CanonicalCommand::SetFeatureBodyOwnership { id, .. } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
                if let Some(feature) = snapshot.feature(*id) {
                    dependencies
                        .insert(AuthoritativeDependency::Definition(feature.definition_id()));
                }
            }
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id,
                body_id,
                suppressed_feature_ids,
            } => {
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                dependencies.insert(AuthoritativeDependency::BodyFeatureSuppression(
                    *definition_id,
                    *body_id,
                ));
                for feature_id in suppressed_feature_ids {
                    add_feature_dependency_closure(snapshot, *feature_id, &mut dependencies);
                }
                if let Some(current) = snapshot.suppressed_feature_ids(*definition_id, *body_id) {
                    for feature_id in current {
                        add_feature_dependency_closure(snapshot, *feature_id, &mut dependencies);
                    }
                }
            }
            CanonicalCommand::CreateFeature {
                id,
                definition_id,
                kind,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Feature(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                match kind {
                    FeatureKind::Workplane(spec) => match &spec.support {
                        WorkplaneSupport::Principal(_) => {}
                        WorkplaneSupport::Offset { base, .. } => {
                            add_feature_dependency_closure(snapshot, *base, &mut dependencies);
                        }
                        WorkplaneSupport::PlanarFace { reference, .. } => {
                            add_feature_dependency_closure(
                                snapshot,
                                reference.producer_feature_id,
                                &mut dependencies,
                            );
                        }
                    },
                    FeatureKind::Sketch(spec) => {
                        add_feature_dependency_closure(snapshot, spec.workplane, &mut dependencies);
                    }
                    FeatureKind::Pad(spec) => {
                        add_feature_dependency_closure(snapshot, spec.sketch, &mut dependencies);
                    }
                    FeatureKind::SketchPocket(spec) => {
                        add_feature_dependency_closure(snapshot, spec.target, &mut dependencies);
                        add_feature_dependency_closure(snapshot, spec.sketch, &mut dependencies);
                    }
                    FeatureKind::Extrusion { profile, .. }
                    | FeatureKind::BottleProfileControl { profile, .. }
                    | FeatureKind::Revolve { profile, .. }
                    | FeatureKind::PlanarOffset { profile, .. } => {
                        add_feature_dependency_closure(snapshot, *profile, &mut dependencies);
                    }
                    FeatureKind::ThroughCut { target, profile }
                    | FeatureKind::Pocket {
                        target, profile, ..
                    } => {
                        add_feature_dependency_closure(snapshot, *target, &mut dependencies);
                        add_feature_dependency_closure(snapshot, *profile, &mut dependencies);
                    }
                    FeatureKind::Sweep { profile, path } => {
                        add_feature_dependency_closure(snapshot, *profile, &mut dependencies);
                        add_feature_dependency_closure(snapshot, *path, &mut dependencies);
                    }
                    FeatureKind::Loft { sections } => {
                        for section in sections {
                            add_feature_dependency_closure(
                                snapshot,
                                section.profile,
                                &mut dependencies,
                            );
                        }
                    }
                    FeatureKind::Boolean { target, tool, .. } => {
                        add_feature_dependency_closure(snapshot, *target, &mut dependencies);
                        add_feature_dependency_closure(snapshot, *tool, &mut dependencies);
                    }
                    FeatureKind::Shell { target, .. }
                    | FeatureKind::BottleEdgeFinish { target, .. }
                    | FeatureKind::TopologyShell { target, .. }
                    | FeatureKind::TopologyEdgeFinish { target, .. }
                    | FeatureKind::TopologyFaceOffset { target, .. } => {
                        add_feature_dependency_closure(snapshot, *target, &mut dependencies);
                    }
                    FeatureKind::Profile { .. }
                    | FeatureKind::SegmentProfile { .. }
                    | FeatureKind::SplineProfile { .. }
                    | FeatureKind::ImportedExactBody(_)
                    | FeatureKind::MeshBody(_) => {}
                }
            }
            CanonicalCommand::DeleteFeature { id } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
                dependencies.insert(AuthoritativeDependency::FeatureUsers(*id));
            }
            CanonicalCommand::SetFeatureDimension { id, .. }
            | CanonicalCommand::SetSketchConstraintDimension { id, .. }
            | CanonicalCommand::TranslateProfile { id, .. }
            | CanonicalCommand::SetBottleControlDimension { id, .. }
            | CanonicalCommand::SetBottleEdgeFinishKind { id, .. }
            | CanonicalCommand::SetProfilePoints { id, .. } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::GuardAssemblyRecompute { .. } => {
                dependencies.extend(
                    snapshot
                        .occurrences()
                        .map(|occurrence| AuthoritativeDependency::Occurrence(occurrence.id())),
                );
                dependencies.extend(
                    snapshot
                        .grounded_occurrences()
                        .map(AuthoritativeDependency::GroundedOccurrence),
                );
                for mate in snapshot.assembly_mates() {
                    dependencies.insert(AuthoritativeDependency::AssemblyMate(mate.id()));
                    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
                        add_feature_dependency_closure(
                            snapshot,
                            endpoint.reference().producer_feature_id,
                            &mut dependencies,
                        );
                    }
                }
                dependencies.extend(
                    snapshot
                        .assembly_joints()
                        .map(|joint| AuthoritativeDependency::AssemblyJoint(joint.id())),
                );
                dependencies.extend(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| AuthoritativeDependency::AssemblyMotionStudy(study.id())),
                );
                dependencies.extend(snapshot.assembly_motion_couplings().map(|coupling| {
                    AuthoritativeDependency::AssemblyMotionCoupling(coupling.id())
                }));
            }
            CanonicalCommand::ApplyAssemblySolve { transforms, .. } => {
                dependencies.extend(
                    transforms
                        .iter()
                        .map(|(id, _)| AuthoritativeDependency::Occurrence(*id)),
                );
                dependencies.extend(
                    snapshot
                        .grounded_occurrences()
                        .map(AuthoritativeDependency::GroundedOccurrence),
                );
                for mate in snapshot.assembly_mates() {
                    dependencies.insert(AuthoritativeDependency::AssemblyMate(mate.id()));
                    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
                        dependencies.insert(AuthoritativeDependency::Occurrence(
                            endpoint.occurrence_id(),
                        ));
                        add_feature_dependency_closure(
                            snapshot,
                            endpoint.reference().producer_feature_id,
                            &mut dependencies,
                        );
                    }
                }
                dependencies.extend(
                    snapshot
                        .assembly_joints()
                        .map(|joint| AuthoritativeDependency::AssemblyJoint(joint.id())),
                );
                dependencies.extend(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| AuthoritativeDependency::AssemblyMotionStudy(study.id())),
                );
                dependencies.extend(snapshot.assembly_motion_couplings().map(|coupling| {
                    AuthoritativeDependency::AssemblyMotionCoupling(coupling.id())
                }));
            }
            CanonicalCommand::SetOccurrenceGrounded { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::GroundedOccurrence(*id));
            }
            CanonicalCommand::CreateAssemblyMate(mate)
            | CanonicalCommand::RebindAssemblyMate(mate) => {
                dependencies.insert(AuthoritativeDependency::AssemblyMate(mate.id()));
                for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
                    dependencies.insert(AuthoritativeDependency::Occurrence(
                        endpoint.occurrence_id(),
                    ));
                    add_feature_dependency_closure(
                        snapshot,
                        endpoint.reference().profile_feature_id,
                        &mut dependencies,
                    );
                    add_feature_dependency_closure(
                        snapshot,
                        endpoint.reference().producer_feature_id,
                        &mut dependencies,
                    );
                }
            }
            CanonicalCommand::SetAssemblyMateKind { id, .. } => {
                dependencies.insert(AuthoritativeDependency::AssemblyMate(*id));
                if let Some(mate) = snapshot.assembly_mate(*id) {
                    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
                        dependencies.insert(AuthoritativeDependency::Occurrence(
                            endpoint.occurrence_id(),
                        ));
                        add_feature_dependency_closure(
                            snapshot,
                            endpoint.reference().producer_feature_id,
                            &mut dependencies,
                        );
                    }
                }
            }
            CanonicalCommand::DeleteAssemblyMate { id } => {
                dependencies.insert(AuthoritativeDependency::AssemblyMate(*id));
            }
            CanonicalCommand::CreateAssemblyJoint(joint) => {
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(joint.id()));
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    joint.parent_occurrence_id(),
                ));
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    joint.child_occurrence_id(),
                ));
                dependencies.extend(
                    snapshot
                        .assembly_joints()
                        .map(|joint| AuthoritativeDependency::AssemblyJoint(joint.id())),
                );
            }
            CanonicalCommand::SetAssemblyJointKind { id, .. }
            | CanonicalCommand::SetAssemblyJointPosition { id, .. }
            | CanonicalCommand::SetAssemblyJointLimits { id, .. } => {
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(*id));
                if let Some(joint) = snapshot.assembly_joint(*id) {
                    dependencies.insert(AuthoritativeDependency::Occurrence(
                        joint.parent_occurrence_id(),
                    ));
                    dependencies.insert(AuthoritativeDependency::Occurrence(
                        joint.child_occurrence_id(),
                    ));
                }
                dependencies.extend(
                    snapshot
                        .assembly_joints()
                        .map(|joint| AuthoritativeDependency::AssemblyJoint(joint.id())),
                );
                dependencies.extend(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| AuthoritativeDependency::AssemblyMotionStudy(study.id())),
                );
                dependencies.extend(snapshot.assembly_motion_couplings().map(|coupling| {
                    AuthoritativeDependency::AssemblyMotionCoupling(coupling.id())
                }));
            }
            CanonicalCommand::DeleteAssemblyJoint { id } => {
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(*id));
                dependencies.extend(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| AuthoritativeDependency::AssemblyMotionStudy(study.id())),
                );
                dependencies.extend(snapshot.assembly_motion_couplings().map(|coupling| {
                    AuthoritativeDependency::AssemblyMotionCoupling(coupling.id())
                }));
            }
            CanonicalCommand::CreateAssemblyMotionCoupling(coupling)
            | CanonicalCommand::UpdateAssemblyMotionCoupling(coupling) => {
                dependencies.insert(AuthoritativeDependency::AssemblyMotionCoupling(
                    coupling.id(),
                ));
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(
                    coupling.input_joint_id(),
                ));
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(
                    coupling.output_joint_id(),
                ));
                dependencies.extend(
                    snapshot
                        .assembly_motion_couplings()
                        .map(|value| AuthoritativeDependency::AssemblyMotionCoupling(value.id())),
                );
            }
            CanonicalCommand::DeleteAssemblyMotionCoupling { id } => {
                dependencies.insert(AuthoritativeDependency::AssemblyMotionCoupling(*id));
            }
            CanonicalCommand::CreateAssemblyMotionStudy(study)
            | CanonicalCommand::UpdateAssemblyMotionStudy(study) => {
                dependencies.insert(AuthoritativeDependency::AssemblyMotionStudy(study.id()));
                dependencies.extend(
                    study
                        .drivers()
                        .iter()
                        .map(|driver| AuthoritativeDependency::AssemblyJoint(driver.joint_id())),
                );
            }
            CanonicalCommand::DeleteAssemblyMotionStudy { id } => {
                dependencies.insert(AuthoritativeDependency::AssemblyMotionStudy(*id));
            }
            CanonicalCommand::CreateDrawingSheet(sheet)
            | CanonicalCommand::UpdateDrawingSheet(sheet) => {
                dependencies.insert(AuthoritativeDependency::DrawingSheet(sheet.id()));
                match sheet.source() {
                    DrawingSource::Definition(id) => {
                        dependencies.insert(AuthoritativeDependency::Definition(*id));
                    }
                    DrawingSource::RigidAssembly { occurrence_ids } => {
                        dependencies.extend(
                            occurrence_ids
                                .iter()
                                .cloned()
                                .map(AuthoritativeDependency::Occurrence),
                        );
                        dependencies.extend(
                            occurrence_ids
                                .iter()
                                .cloned()
                                .map(AuthoritativeDependency::GroundedOccurrence),
                        );
                        dependencies.extend(
                            snapshot
                                .assembly_mates()
                                .filter(|mate| {
                                    occurrence_ids.contains(&mate.endpoint_a().occurrence_id())
                                        || occurrence_ids
                                            .contains(&mate.endpoint_b().occurrence_id())
                                })
                                .map(|mate| AuthoritativeDependency::AssemblyMate(mate.id())),
                        );
                    }
                }
            }
            CanonicalCommand::DeleteDrawingSheet { id } => {
                dependencies.insert(AuthoritativeDependency::DrawingSheet(*id));
            }
            CanonicalCommand::CreateOccurrence {
                id,
                definition_id,
                parent,
                tag,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                if let Some(tag_id) = tag {
                    dependencies.insert(AuthoritativeDependency::Tag(*tag_id));
                }
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::DeleteOccurrence { id } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::OccurrenceCollections(*id));
            }
            CanonicalCommand::SetOccurrenceTransform { id, .. }
            | CanonicalCommand::RenameEntity { id, .. }
            | CanonicalCommand::SetOccurrenceVisibility { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
            }
            CanonicalCommand::SetOccurrenceTag { id, tag } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                if let Some(tag_id) = tag {
                    dependencies.insert(AuthoritativeDependency::Tag(*tag_id));
                }
            }
            CanonicalCommand::RepointOccurrence { id, definition_id } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::SetOccurrenceParent { id, parent } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::CreateGroup { id, parent, .. } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::DeleteGroup { id } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                dependencies.insert(AuthoritativeDependency::GroupChildren(*id));
            }
            CanonicalCommand::SetGroupTransform { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
            }
            CanonicalCommand::SetGroupParent { id, parent } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                dependencies.insert(AuthoritativeDependency::Occurrence(plan.occurrence_id));
                dependencies.insert(AuthoritativeDependency::Definition(
                    plan.source_definition_id,
                ));
                dependencies.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                for (source_id, new_id) in &plan.feature_id_map {
                    add_feature_dependency_closure(snapshot, *source_id, &mut dependencies);
                    dependencies.insert(AuthoritativeDependency::FeatureParameterBindings(
                        *source_id,
                    ));
                    dependencies.insert(AuthoritativeDependency::Feature(*new_id));
                }
                if let Some(definition) = snapshot.definition(plan.source_definition_id) {
                    for local_id in definition.local_group_ids() {
                        dependencies.insert(AuthoritativeDependency::LocalGroup(LocalGroupKey {
                            definition_id: plan.source_definition_id,
                            local_id: *local_id,
                        }));
                    }
                    for local_id in definition.local_occurrence_ids() {
                        dependencies.insert(AuthoritativeDependency::LocalOccurrence(
                            LocalOccurrenceKey {
                                definition_id: plan.source_definition_id,
                                local_id: *local_id,
                            },
                        ));
                    }
                }
            }
            CanonicalCommand::ConvertGroupToComponent(plan) => {
                dependencies.insert(AuthoritativeDependency::GroupSubtree(plan.group_id));
                dependencies.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                dependencies.insert(AuthoritativeDependency::Occurrence(plan.new_occurrence_id));
            }
            CanonicalCommand::ApplySolidTool(plan) => {
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    plan.target_occurrence_id,
                ));
                dependencies.insert(AuthoritativeDependency::Occurrence(plan.tool_occurrence_id));
                dependencies.insert(AuthoritativeDependency::OccurrenceCollections(
                    plan.tool_occurrence_id,
                ));
                add_feature_dependency_closure(snapshot, plan.target_feature_id, &mut dependencies);
                add_feature_dependency_closure(snapshot, plan.tool_feature_id, &mut dependencies);
                dependencies.insert(AuthoritativeDependency::Definition(
                    plan.result_definition_id,
                ));
                dependencies.extend(
                    plan.result_feature_ids
                        .iter()
                        .cloned()
                        .map(AuthoritativeDependency::Feature),
                );
            }
            CanonicalCommand::CreateExpressionNode { id, expression, .. } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
                if let Ok(expression) = ExpressionAst::parse(expression) {
                    for dependency in expression.dependencies() {
                        add_evaluator_dependency_closure(snapshot, dependency, &mut dependencies);
                    }
                }
            }
            CanonicalCommand::CreateRuleNode { id, expression, .. } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
                if let Ok(expression) = ExpressionAst::parse(expression) {
                    for dependency in expression.dependencies() {
                        add_evaluator_dependency_closure(snapshot, dependency, &mut dependencies);
                    }
                }
            }
            CanonicalCommand::SetNodeExpression { id, expression } => {
                add_evaluator_dependency_closure(snapshot, *id, &mut dependencies);
                if let Ok(expression) = ExpressionAst::parse(expression) {
                    for dependency in expression.dependencies() {
                        add_evaluator_dependency_closure(snapshot, dependency, &mut dependencies);
                    }
                }
            }
            CanonicalCommand::SetRuleOutputs { id, .. } => {
                add_evaluator_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::UpsertOverride(value) => {
                dependencies.insert(AuthoritativeDependency::Override(value.id));
                add_evaluator_dependency_closure(
                    snapshot,
                    value.target.root_rule_node_id,
                    &mut dependencies,
                );
            }
            CanonicalCommand::DeleteOverride { id } => {
                dependencies.insert(AuthoritativeDependency::Override(*id));
            }
            CanonicalCommand::UpsertFeatureParameterBinding(binding) => {
                dependencies.insert(AuthoritativeDependency::FeatureParameterBinding(
                    binding.target.clone(),
                ));
                add_feature_dependency_closure(
                    snapshot,
                    binding.target.feature_id,
                    &mut dependencies,
                );
                add_evaluator_dependency_closure(
                    snapshot,
                    binding.derived_from.root_rule_node_id,
                    &mut dependencies,
                );
            }
            CanonicalCommand::DeleteFeatureParameterBinding { target } => {
                dependencies.insert(AuthoritativeDependency::FeatureParameterBinding(
                    target.clone(),
                ));
            }
            CanonicalCommand::RecomputeFeatureParameters { .. } => {
                for binding in snapshot.feature_parameter_bindings() {
                    dependencies.insert(AuthoritativeDependency::FeatureParameterBinding(
                        binding.target.clone(),
                    ));
                    add_feature_dependency_closure(
                        snapshot,
                        binding.target.feature_id,
                        &mut dependencies,
                    );
                    add_evaluator_dependency_closure(
                        snapshot,
                        binding.derived_from.root_rule_node_id,
                        &mut dependencies,
                    );
                }
            }
            CanonicalCommand::UpsertJoint(joint) => {
                dependencies.insert(AuthoritativeDependency::Joint(joint.id()));
                add_evaluator_dependency_closure(
                    snapshot,
                    joint.participant_a().root_rule_node_id,
                    &mut dependencies,
                );
                add_evaluator_dependency_closure(
                    snapshot,
                    joint.participant_b().root_rule_node_id,
                    &mut dependencies,
                );
            }
            CanonicalCommand::DeleteJoint { id } => {
                dependencies.insert(AuthoritativeDependency::Joint(*id));
            }
            CanonicalCommand::UpsertSpace(space) => {
                dependencies.insert(AuthoritativeDependency::Space(space.id()));
                dependencies.extend(
                    space
                        .adjacent_to()
                        .iter()
                        .chain(space.accessible_to())
                        .cloned()
                        .map(AuthoritativeDependency::Space),
                );
            }
            CanonicalCommand::DeleteSpace { id } => {
                dependencies.insert(AuthoritativeDependency::Space(*id));
            }
            CanonicalCommand::UpsertClearanceVolume(clearance) => {
                dependencies.insert(AuthoritativeDependency::ClearanceVolume(clearance.id()));
                match clearance.owner() {
                    ClearanceOwner::Occurrence(path) => {
                        dependencies
                            .insert(AuthoritativeDependency::Occurrence(path.root_occurrence()));
                    }
                    ClearanceOwner::Space(id) => {
                        dependencies.insert(AuthoritativeDependency::Space(*id));
                    }
                }
                if let Some(identity) = clearance.derived_from() {
                    add_evaluator_dependency_closure(
                        snapshot,
                        identity.root_rule_node_id,
                        &mut dependencies,
                    );
                }
            }
            CanonicalCommand::DeleteClearanceVolume { id } => {
                dependencies.insert(AuthoritativeDependency::ClearanceVolume(*id));
            }
            CanonicalCommand::UpsertPersistentDimension(dimension) => {
                dependencies.insert(AuthoritativeDependency::PersistentDimension(dimension.id));
                match &dimension.target {
                    PersistentDimensionTarget::FeatureParameter(target) => {
                        add_feature_dependency_closure(
                            snapshot,
                            target.feature_id,
                            &mut dependencies,
                        );
                    }
                    PersistentDimensionTarget::DerivedOutput(target) => {
                        add_evaluator_dependency_closure(
                            snapshot,
                            target.root_rule_node_id,
                            &mut dependencies,
                        );
                    }
                    PersistentDimensionTarget::ExactFeatureParameter { .. } => {}
                }
            }
            CanonicalCommand::DeletePersistentDimension { id } => {
                dependencies.insert(AuthoritativeDependency::PersistentDimension(*id));
            }
            CanonicalCommand::CreateTag { id, .. }
            | CanonicalCommand::DeleteTag { id }
            | CanonicalCommand::SetTagVisibility { id, .. }
            | CanonicalCommand::SetTagName { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Tag(*id));
            }
            CanonicalCommand::UpsertClassificationDimension { id, .. } => {
                dependencies.insert(AuthoritativeDependency::ClassificationDimension(*id));
                dependencies.extend(
                    snapshot
                        .product
                        .classification_assignments
                        .keys()
                        .filter(|(_, dimension_id)| dimension_id == id)
                        .map(|(occurrence_id, dimension_id)| {
                            AuthoritativeDependency::OccurrenceClassification(
                                *occurrence_id,
                                *dimension_id,
                            )
                        }),
                );
            }
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id,
                dimension_id,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*occurrence_id));
                dependencies.insert(AuthoritativeDependency::ClassificationDimension(
                    *dimension_id,
                ));
                dependencies.insert(AuthoritativeDependency::OccurrenceClassification(
                    *occurrence_id,
                    *dimension_id,
                ));
            }
            CanonicalCommand::CreateCollection { id, .. }
            | CanonicalCommand::DeleteCollection { id } => {
                dependencies.insert(AuthoritativeDependency::Collection(*id));
            }
            CanonicalCommand::SetCollectionOccurrences { id, occurrence_ids } => {
                dependencies.insert(AuthoritativeDependency::Collection(*id));
                dependencies.extend(
                    occurrence_ids
                        .iter()
                        .cloned()
                        .map(AuthoritativeDependency::Occurrence),
                );
            }
        }
    }
    dependencies
}

fn add_evaluator_dependency_closure(
    snapshot: &Snapshot,
    id: NodeId,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    if !dependencies.insert(AuthoritativeDependency::EvaluatorNode(id)) {
        return;
    }
    if let Some(node) = snapshot.evaluator_node(id) {
        for dependency in node.dependencies() {
            add_evaluator_dependency_closure(snapshot, *dependency, dependencies);
        }
    }
}

fn add_feature_dependency_closure(
    snapshot: &Snapshot,
    id: FeatureId,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    if !dependencies.insert(AuthoritativeDependency::Feature(id)) {
        return;
    }
    if let Some(feature) = snapshot.feature(id) {
        dependencies.insert(AuthoritativeDependency::Definition(feature.definition_id()));
        match feature.kind() {
            FeatureKind::Workplane(spec) => match &spec.support {
                WorkplaneSupport::Principal(_) => {}
                WorkplaneSupport::Offset { base, .. } => {
                    add_feature_dependency_closure(snapshot, *base, dependencies);
                }
                WorkplaneSupport::PlanarFace { reference, .. } => {
                    add_feature_dependency_closure(
                        snapshot,
                        reference.producer_feature_id,
                        dependencies,
                    );
                }
            },
            FeatureKind::Sketch(spec) => {
                add_feature_dependency_closure(snapshot, spec.workplane, dependencies);
            }
            FeatureKind::Pad(spec) => {
                add_feature_dependency_closure(snapshot, spec.sketch, dependencies);
            }
            FeatureKind::SketchPocket(spec) => {
                add_feature_dependency_closure(snapshot, spec.target, dependencies);
                add_feature_dependency_closure(snapshot, spec.sketch, dependencies);
            }
            FeatureKind::Extrusion { profile, .. }
            | FeatureKind::BottleProfileControl { profile, .. }
            | FeatureKind::Revolve { profile, .. }
            | FeatureKind::PlanarOffset { profile, .. } => {
                add_feature_dependency_closure(snapshot, *profile, dependencies);
            }
            FeatureKind::ThroughCut { target, profile }
            | FeatureKind::Pocket {
                target, profile, ..
            } => {
                add_feature_dependency_closure(snapshot, *target, dependencies);
                add_feature_dependency_closure(snapshot, *profile, dependencies);
            }
            FeatureKind::Sweep { profile, path } => {
                add_feature_dependency_closure(snapshot, *profile, dependencies);
                add_feature_dependency_closure(snapshot, *path, dependencies);
            }
            FeatureKind::Loft { sections } => {
                for section in sections {
                    add_feature_dependency_closure(snapshot, section.profile, dependencies);
                }
            }
            FeatureKind::Boolean { target, tool, .. } => {
                add_feature_dependency_closure(snapshot, *target, dependencies);
                add_feature_dependency_closure(snapshot, *tool, dependencies);
            }
            FeatureKind::Shell { target, .. }
            | FeatureKind::BottleEdgeFinish { target, .. }
            | FeatureKind::TopologyShell { target, .. }
            | FeatureKind::TopologyEdgeFinish { target, .. }
            | FeatureKind::TopologyFaceOffset { target, .. } => {
                add_feature_dependency_closure(snapshot, *target, dependencies);
            }
            FeatureKind::Profile { .. }
            | FeatureKind::SegmentProfile { .. }
            | FeatureKind::SplineProfile { .. }
            | FeatureKind::ImportedExactBody(_)
            | FeatureKind::MeshBody(_) => {}
        }
    }
}

fn add_group_ancestry(
    snapshot: &Snapshot,
    mut group_id: Option<GroupId>,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    while let Some(id) = group_id {
        if !dependencies.insert(AuthoritativeDependency::Group(id)) {
            break;
        }
        group_id = snapshot.group(id).and_then(Group::parent);
    }
}

fn dependency_digest(
    snapshot: &Snapshot,
    dependencies: &BTreeSet<AuthoritativeDependency>,
) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.authoritative-dependencies.v1");
    digest.u64(dependencies.len() as u64);
    for dependency in dependencies {
        digest.authoritative_dependency(snapshot.product(), dependency.clone());
    }
    digest.finish()
}

fn dependent_closure(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
    changed: &BTreeSet<NodeId>,
) -> BTreeSet<NodeId> {
    let mut closure = changed.clone();
    loop {
        let before = closure.len();
        for (id, node) in nodes {
            if node
                .dependencies
                .iter()
                .any(|dependency| closure.contains(dependency))
            {
                closure.insert(*id);
            }
        }
        if closure.len() == before {
            return closure;
        }
    }
}

pub(crate) fn validate_graph(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
) -> Result<(), CanonicalError> {
    validate_typed_graph(nodes).map_err(CanonicalError::Graph)
}

fn digest_snapshot(snapshot: &Snapshot) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.document.v3");
    digest.u64(snapshot.product.document_id.0);
    digest.byte(match snapshot.product.units {
        UnitSystem::Millimetres => 1,
    });
    digest.u64(snapshot.product.evaluator_nodes.len() as u64);
    for node in snapshot.product.evaluator_nodes.values() {
        digest.node(node);
    }
    digest.u64(snapshot.product.overrides.len() as u64);
    for value in snapshot.product.overrides.values() {
        digest.canonical_override(value);
    }
    digest.u64(snapshot.product.feature_parameter_bindings.len() as u64);
    for binding in snapshot.product.feature_parameter_bindings.values() {
        digest.feature_parameter_binding(binding);
    }
    digest.u64(snapshot.product.joints.len() as u64);
    for joint in snapshot.product.joints.values() {
        digest.joint(joint);
    }
    digest.u64(snapshot.product.spaces.len() as u64);
    for space in snapshot.product.spaces.values() {
        digest.space(space);
    }
    digest.u64(snapshot.product.clearance_volumes.len() as u64);
    for clearance in snapshot.product.clearance_volumes.values() {
        digest.clearance_volume(clearance);
    }
    digest.u64(snapshot.product.persistent_dimensions.len() as u64);
    for dimension in snapshot.product.persistent_dimensions.values() {
        digest.persistent_dimension(dimension);
    }
    digest.u64(snapshot.product.tags.len() as u64);
    for tag in snapshot.product.tags.values() {
        digest.tag(tag);
    }
    digest.u64(snapshot.product.classification_dimensions.len() as u64);
    for dimension in snapshot.product.classification_dimensions.values() {
        digest.u64(dimension.id.0);
        digest.bytes(dimension.name.as_bytes());
        digest.u64(dimension.categories.len() as u64);
        for category in dimension.categories.values() {
            digest.u64(category.id.0);
            digest.bytes(category.name.as_bytes());
        }
    }
    digest.u64(snapshot.product.classification_assignments.len() as u64);
    for ((occurrence_id, dimension_id), category_id) in &snapshot.product.classification_assignments
    {
        digest.u64(occurrence_id.0);
        digest.u64(dimension_id.0);
        digest.u64(category_id.0);
    }
    digest.u64(snapshot.product.collections.len() as u64);
    for collection in snapshot.product.collections.values() {
        digest.collection(collection);
    }
    digest.u64(snapshot.product.import_receipts.len() as u64);
    for receipt in snapshot.product.import_receipts.values() {
        digest.import_receipt(receipt);
    }
    digest.u64(snapshot.product.definitions.len() as u64);
    for definition in snapshot.product.definitions.values() {
        digest.definition(definition);
    }
    digest.u64(snapshot.product.features.len() as u64);
    for feature in snapshot.product.features.values() {
        digest.feature(feature);
    }
    digest.u64(snapshot.product.body_feature_suppression.len() as u64);
    for ((definition_id, body_id), suppressed) in &snapshot.product.body_feature_suppression {
        digest.u64(definition_id.0);
        digest.u64(body_id.0);
        digest.u64(suppressed.len() as u64);
        for feature_id in suppressed {
            digest.u64(feature_id.0);
        }
    }
    digest.u64(snapshot.product.occurrences.len() as u64);
    for occurrence in snapshot.product.occurrences.values() {
        digest.occurrence(occurrence);
    }
    digest.u64(snapshot.product.grounded_occurrences.len() as u64);
    for occurrence_id in &snapshot.product.grounded_occurrences {
        digest.u64(occurrence_id.0);
    }
    digest.u64(snapshot.product.assembly_mates.len() as u64);
    for mate in snapshot.product.assembly_mates.values() {
        digest.assembly_mate(mate);
    }
    if !snapshot.product.assembly_joints.is_empty() {
        digest.bytes(b"canonical-assembly-joints.v1");
        digest.u64(snapshot.product.assembly_joints.len() as u64);
        for joint in snapshot.product.assembly_joints.values() {
            digest.assembly_joint(joint);
        }
    }
    if !snapshot.product.assembly_motion_couplings.is_empty() {
        digest.bytes(b"canonical-assembly-motion-couplings.v1");
        digest.u64(snapshot.product.assembly_motion_couplings.len() as u64);
        for coupling in snapshot.product.assembly_motion_couplings.values() {
            digest.assembly_motion_coupling(coupling);
        }
    }
    if !snapshot.product.assembly_motion_studies.is_empty() {
        digest.bytes(b"canonical-assembly-motion-studies.v1");
        digest.u64(snapshot.product.assembly_motion_studies.len() as u64);
        for study in snapshot.product.assembly_motion_studies.values() {
            digest.assembly_motion_study(study);
        }
    }
    if !snapshot.product.drawing_sheets.is_empty() {
        digest.bytes(b"canonical-drawing-sheets.v1");
        digest.u64(snapshot.product.drawing_sheets.len() as u64);
        for sheet in snapshot.product.drawing_sheets.values() {
            digest.drawing_sheet(sheet);
        }
    }
    digest.u64(snapshot.product.groups.len() as u64);
    for group in snapshot.product.groups.values() {
        digest.group(group);
    }
    digest.u64(snapshot.product.local_groups.len() as u64);
    for group in snapshot.product.local_groups.values() {
        digest.local_group(group);
    }
    digest.u64(snapshot.product.local_occurrences.len() as u64);
    for occurrence in snapshot.product.local_occurrences.values() {
        digest.local_occurrence(occurrence);
    }
    digest.finish()
}

struct StableDigest(u64);

impl StableDigest {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    const fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(Self::PRIME);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        for byte in bytes {
            self.byte(*byte);
        }
    }

    fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    fn node(&mut self, node: &EvaluatorNode) {
        self.bytes(&node.canonical_spec_bytes());
    }

    fn ports(&mut self, ports: &[PortSpec]) {
        self.u64(ports.len() as u64);
        for port in ports {
            self.bytes(port.name().as_bytes());
            self.byte(match port.value_type() {
                ValueType::Number => 1,
            });
        }
    }
    fn rule_outputs(&mut self, outputs: &[RuleOutput]) {
        self.u64(outputs.len() as u64);
        let mut stack = outputs.iter().rev().collect::<Vec<_>>();
        while let Some(output) = stack.pop() {
            let segment = output.segment();
            self.u64(segment.producer_rule_id.0);
            self.bytes(segment.output_port.as_bytes());
            self.bytes(segment.semantic_key.as_bytes());
            self.u64(output.children().len() as u64);
            stack.extend(output.children().iter().rev());
        }
    }

    fn slot_path(&mut self, path: &SlotPath) {
        self.u64(path.segments().len() as u64);
        for segment in path.segments() {
            self.u64(segment.producer_rule_id.0);
            self.bytes(segment.output_port.as_bytes());
            self.bytes(segment.semantic_key.as_bytes());
        }
    }

    fn canonical_override(&mut self, value: &CanonicalOverride) {
        self.u64(value.id);
        self.u64(value.target.root_rule_node_id.0);
        self.slot_path(&value.target.slot_path);
        self.bytes(value.parameter.as_bytes());
        self.u64(value.value_bits);
        match value.health {
            SlotResolution::Resolved => self.byte(1),
            SlotResolution::Ambiguous { segment_index } => {
                self.byte(2);
                self.u64(segment_index as u64);
            }
            SlotResolution::Lost { segment_index } => {
                self.byte(3);
                self.u64(segment_index as u64);
            }
        }
    }

    fn feature_parameter_target(&mut self, target: &FeatureParameterTarget) {
        self.u64(target.feature_id.0);
        self.bytes(target.path.as_str().as_bytes());
        self.byte(match target.value_type {
            ParameterValueType::Length => 1,
            ParameterValueType::Angle => 2,
            ParameterValueType::Scalar => 3,
        });
    }

    fn feature_parameter_binding(&mut self, binding: &FeatureParameterBinding) {
        self.feature_parameter_target(&binding.target);
        self.u64(binding.derived_from.root_rule_node_id.0);
        self.slot_path(&binding.derived_from.slot_path);
    }

    fn joint(&mut self, joint: &CanonicalJoint) {
        self.u64(joint.id().0);
        self.u64(joint.participant_a().root_rule_node_id.0);
        self.slot_path(&joint.participant_a().slot_path);
        self.u64(joint.participant_b().root_rule_node_id.0);
        self.slot_path(&joint.participant_b().slot_path);
        for value in joint.volume().min().into_iter().chain(joint.volume().max()) {
            self.u64(value.to_bits());
        }
    }

    fn space(&mut self, space: &CanonicalSpace) {
        self.u64(space.id().0);
        self.bytes(space.purpose().as_bytes());
        for value in space.volume().min().into_iter().chain(space.volume().max()) {
            self.u64(value.to_bits());
        }
        self.u64(space.adjacent_to().len() as u64);
        for id in space.adjacent_to() {
            self.u64(id.0);
        }
        self.u64(space.accessible_to().len() as u64);
        for id in space.accessible_to() {
            self.u64(id.0);
        }
    }

    fn clearance_volume(&mut self, clearance: &CanonicalClearanceVolume) {
        self.u64(clearance.id().0);
        match clearance.owner() {
            ClearanceOwner::Occurrence(path) => {
                self.byte(1);
                self.u64(path.root_occurrence().0);
                self.u64(path.steps().len() as u64);
                for step in path.steps() {
                    match step {
                        InstancePathStep::Group(id) => {
                            self.byte(1);
                            self.u64(id.0);
                        }
                        InstancePathStep::Occurrence(id) => {
                            self.byte(2);
                            self.u64(id.0);
                        }
                    }
                }
            }
            ClearanceOwner::Space(id) => {
                self.byte(2);
                self.u64(id.0);
            }
        }
        self.bytes(clearance.reason().as_bytes());
        for value in clearance
            .volume()
            .min()
            .into_iter()
            .chain(clearance.volume().max())
        {
            self.u64(value.to_bits());
        }
        self.byte(match clearance.coordinate_frame() {
            ClearanceCoordinateFrame::World => 1,
        });
        self.u64(clearance.tolerance().epsilon_mm().to_bits());
        self.byte(match clearance.severity() {
            ClearanceSeverity::Advisory => 1,
            ClearanceSeverity::Required => 2,
        });
        if let Some(identity) = clearance.derived_from() {
            self.byte(1);
            self.u64(identity.root_rule_node_id.0);
            self.slot_path(&identity.slot_path);
        } else {
            self.byte(0);
        }
    }

    fn persistent_dimension(&mut self, dimension: &PersistentDimension) {
        self.u64(dimension.id.0);
        self.bytes(dimension.name.as_bytes());
        match &dimension.target {
            PersistentDimensionTarget::FeatureParameter(target) => {
                self.byte(1);
                self.feature_parameter_target(target);
            }
            PersistentDimensionTarget::DerivedOutput(target) => {
                self.byte(2);
                self.u64(target.root_rule_node_id.0);
                self.slot_path(&target.slot_path);
            }
            PersistentDimensionTarget::ExactFeatureParameter {
                definition_id,
                producer_feature_id,
                semantic_role,
                source_element_id,
                path,
                value_type,
            } => {
                self.byte(3);
                self.u64(definition_id.0);
                self.feature_parameter_target(&FeatureParameterTarget {
                    feature_id: *producer_feature_id,
                    path: path.clone(),
                    value_type: *value_type,
                });
                self.bytes(semantic_role.as_bytes());
                self.bytes(source_element_id.as_bytes());
            }
        }
        self.byte(match dimension.presentation.unit {
            DimensionDisplayUnit::Millimetres => 1,
            DimensionDisplayUnit::Centimetres => 2,
            DimensionDisplayUnit::Inches => 3,
        });
        self.byte(dimension.presentation.decimal_places);
    }

    fn tag(&mut self, tag: &Tag) {
        self.u64(tag.id.0);
        self.bytes(tag.name.as_bytes());
        self.byte(u8::from(tag.visible));
    }

    fn collection(&mut self, collection: &Collection) {
        self.u64(collection.id.0);
        self.bytes(collection.name.as_bytes());
        self.u64(collection.occurrence_ids.len() as u64);
        for occurrence_id in &collection.occurrence_ids {
            self.u64(occurrence_id.0);
        }
    }

    fn import_receipt(&mut self, receipt: &ImportReceipt) {
        self.bytes(receipt.schema().as_bytes());
        self.u64(receipt.id().0);
        self.byte(match receipt.format() {
            ImportFormat::Stl => 1,
            ImportFormat::Dxf => 2,
            ImportFormat::Step => 3,
            ImportFormat::SketchupScene => 4,
        });
        self.bytes(receipt.source_sha256());
        self.u64(receipt.source_byte_len());
        self.bytes(receipt.source_name().as_bytes());
        self.byte(match receipt.units().source_unit() {
            ImportLengthUnit::Millimetre => 1,
            ImportLengthUnit::Centimetre => 2,
            ImportLengthUnit::Metre => 3,
            ImportLengthUnit::Inch => 4,
            ImportLengthUnit::Foot => 5,
        });
        self.byte(match receipt.units().authority() {
            ImportUnitAuthority::FileDeclared => 1,
            ImportUnitAuthority::UserDeclared => 2,
        });
        self.bytes(receipt.parser_id().as_bytes());
        self.bytes(receipt.parser_version().as_bytes());
        self.u64(receipt.diagnostics().len() as u64);
        for diagnostic in receipt.diagnostics() {
            self.byte(match diagnostic.severity() {
                ImportDiagnosticSeverity::Info => 1,
                ImportDiagnosticSeverity::Warning => 2,
            });
            self.bytes(diagnostic.code().as_bytes());
            match diagnostic.subject() {
                Some(subject) => {
                    self.byte(1);
                    self.bytes(subject.as_bytes());
                }
                None => self.byte(0),
            }
            self.u64(u64::from(diagnostic.count()));
        }
        self.u64(receipt.outputs().len() as u64);
        for output in receipt.outputs() {
            match output {
                ImportOutputRef::Definition(id) => {
                    self.byte(1);
                    self.u64(id.0);
                }
                ImportOutputRef::Feature(id) => {
                    self.byte(2);
                    self.u64(id.0);
                }
                ImportOutputRef::Occurrence(id) => {
                    self.byte(3);
                    self.u64(id.0);
                }
            }
        }
    }

    fn transform(&mut self, transform: Transform) {
        for value in transform.matrix {
            self.u64(value.to_bits());
        }
    }

    fn definition(&mut self, definition: &Definition) {
        self.u64(definition.id.0);
        self.bytes(definition.name.as_bytes());
        self.u64(definition.feature_ids.len() as u64);
        for feature_id in &definition.feature_ids {
            self.u64(feature_id.0);
        }
        self.u64(definition.bodies.len() as u64);
        for body in definition.bodies.values() {
            self.u64(body.id.0);
            self.bytes(body.name.as_bytes());
            self.byte(u8::from(body.visible));
            self.optional_id(body.consumed_by.map(|id| id.0));
        }
        self.u64(definition.active_body_id.0);
        self.u64(definition.feature_body_ownership.len() as u64);
        for (feature_id, ownership) in &definition.feature_body_ownership {
            self.u64(feature_id.0);
            self.u64(ownership.input_body_ids.len() as u64);
            for body_id in &ownership.input_body_ids {
                self.u64(body_id.0);
            }
            self.optional_id(ownership.output_body_id.map(|body_id| body_id.0));
        }
        self.u64(definition.local_group_ids.len() as u64);
        for id in &definition.local_group_ids {
            self.u64(id.0);
        }
        self.u64(definition.local_occurrence_ids.len() as u64);
        for id in &definition.local_occurrence_ids {
            self.u64(id.0);
        }
    }

    fn sketch_point_ref(&mut self, reference: crate::sketch::SketchPointRef) {
        self.u64(reference.entity.0);
        self.byte(match reference.point {
            SketchPointKind::Start => 1,
            SketchPointKind::End => 2,
            SketchPointKind::Center => 3,
        });
    }

    fn body_subshape_reference(&mut self, reference: &BodySubshapeRef) {
        self.u64(reference.document_id.0);
        self.u64(reference.definition_id.0);
        self.u64(reference.profile_feature_id.0);
        self.u64(reference.producer_feature_id.0);
        self.bytes(reference.semantic_role.as_bytes());
        self.bytes(reference.source_element_id.as_bytes());
        self.bytes(reference.expected_type.as_bytes());
        self.u64(u64::from(reference.expected_cardinality));
        self.byte(match reference.stability {
            crate::exact_product::ReferenceStability::Guaranteed => 1,
        });
        self.bytes(reference.lineage_digest.as_bytes());
    }

    fn topological_reference(&mut self, reference: &TopologicalElementRef) {
        self.bytes(
            &reference
                .to_bytes()
                .expect("validated topological feature reference is serializable"),
        );
    }

    fn feature_direction(&mut self, direction: crate::sketch::FeatureDirection) {
        match direction {
            crate::sketch::FeatureDirection::AlongNormal => self.byte(1),
            crate::sketch::FeatureDirection::OppositeNormal => self.byte(2),
            crate::sketch::FeatureDirection::Vector(vector) => {
                self.byte(3);
                for component in vector {
                    self.u64(component.to_bits());
                }
            }
        }
    }

    fn feature_extent_end(&mut self, end: &crate::sketch::FeatureExtentEnd) {
        match end {
            crate::sketch::FeatureExtentEnd::Blind(distance) => {
                self.byte(1);
                self.bytes(distance.source_token().as_bytes());
                self.u64(distance.millimetres().to_bits());
            }
            crate::sketch::FeatureExtentEnd::ThroughAll => self.byte(2),
            crate::sketch::FeatureExtentEnd::UpToFace(reference) => {
                self.byte(3);
                self.body_subshape_reference(reference);
            }
        }
    }

    fn feature_extent(&mut self, extent: &crate::sketch::FeatureExtent) {
        match extent {
            crate::sketch::FeatureExtent::Blind(distance) => {
                self.byte(1);
                self.bytes(distance.source_token().as_bytes());
                self.u64(distance.millimetres().to_bits());
            }
            crate::sketch::FeatureExtent::ThroughAll => self.byte(2),
            crate::sketch::FeatureExtent::UpToFace(reference) => {
                self.byte(3);
                self.body_subshape_reference(reference);
            }
            crate::sketch::FeatureExtent::Symmetric(distance) => {
                self.byte(4);
                self.bytes(distance.source_token().as_bytes());
                self.u64(distance.millimetres().to_bits());
            }
            crate::sketch::FeatureExtent::Bidirectional { along, opposite } => {
                self.byte(5);
                self.feature_extent_end(along);
                self.feature_extent_end(opposite);
            }
        }
    }

    fn feature_kind(&mut self, kind: &FeatureKind) {
        match kind {
            FeatureKind::Workplane(spec) => {
                self.byte(17);
                match &spec.support {
                    WorkplaneSupport::Principal(plane) => {
                        self.byte(1);
                        self.byte(match plane {
                            PrincipalPlane::Xy => 1,
                            PrincipalPlane::Yz => 2,
                            PrincipalPlane::Xz => 3,
                        });
                    }
                    WorkplaneSupport::Offset { base, distance } => {
                        self.byte(2);
                        self.u64(base.0);
                        self.bytes(distance.source_token().as_bytes());
                        self.u64(distance.millimetres().to_bits());
                    }
                    WorkplaneSupport::PlanarFace { reference, .. } => {
                        self.byte(3);
                        self.body_subshape_reference(reference);
                    }
                }
                if !matches!(&spec.support, WorkplaneSupport::PlanarFace { .. }) {
                    for coordinate in spec
                        .frame
                        .origin_mm
                        .iter()
                        .chain(spec.frame.x_axis.iter())
                        .chain(spec.frame.y_axis.iter())
                        .chain(spec.frame.normal.iter())
                    {
                        self.u64(coordinate.to_bits());
                    }
                }
            }
            FeatureKind::Sketch(spec) => {
                self.byte(18);
                self.u64(spec.workplane.0);
                self.u64(spec.entities.len() as u64);
                for entity in &spec.entities {
                    match entity {
                        SketchEntity::Line {
                            id,
                            start_mm,
                            end_mm,
                        } => {
                            self.byte(1);
                            self.u64(id.0);
                            for point in [start_mm, end_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                        }
                        SketchEntity::Arc {
                            id,
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        } => {
                            self.byte(2);
                            self.u64(id.0);
                            for point in [start_mm, end_mm, center_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                            self.byte(u8::from(*clockwise));
                        }
                        SketchEntity::Circle {
                            id,
                            center_mm,
                            radius_mm,
                        } => {
                            self.byte(3);
                            self.u64(id.0);
                            self.u64(center_mm[0].to_bits());
                            self.u64(center_mm[1].to_bits());
                            self.u64(radius_mm.to_bits());
                        }
                    }
                }
                self.u64(spec.constraints.len() as u64);
                for constraint in &spec.constraints {
                    self.u64(constraint.id.0);
                    match &constraint.kind {
                        SketchConstraintKind::Horizontal { entity } => {
                            self.byte(1);
                            self.u64(entity.0);
                        }
                        SketchConstraintKind::Vertical { entity } => {
                            self.byte(2);
                            self.u64(entity.0);
                        }
                        SketchConstraintKind::Coincident { a, b } => {
                            self.byte(3);
                            self.sketch_point_ref(*a);
                            self.sketch_point_ref(*b);
                        }
                        SketchConstraintKind::Distance { a, b, value } => {
                            self.byte(4);
                            self.sketch_point_ref(*a);
                            self.sketch_point_ref(*b);
                            self.bytes(value.source_token().as_bytes());
                            self.u64(value.millimetres().to_bits());
                        }
                        SketchConstraintKind::Radius { entity, value } => {
                            self.byte(5);
                            self.u64(entity.0);
                            self.bytes(value.source_token().as_bytes());
                            self.u64(value.millimetres().to_bits());
                        }
                        SketchConstraintKind::FixedPoint { point, position_mm } => {
                            self.byte(6);
                            self.sketch_point_ref(*point);
                            self.u64(position_mm[0].to_bits());
                            self.u64(position_mm[1].to_bits());
                        }
                    }
                }
            }
            FeatureKind::Profile { points_mm } => {
                self.byte(1);
                self.u64(points_mm.len() as u64);
                for point in points_mm {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            FeatureKind::SegmentProfile { segments, closed } => {
                self.byte(11);
                self.byte(u8::from(*closed));
                self.u64(segments.len() as u64);
                for segment in segments {
                    match segment {
                        ProfileSegment::Line { start_mm, end_mm } => {
                            self.byte(1);
                            for point in [start_mm, end_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                        }
                        ProfileSegment::CircularArc {
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        } => {
                            self.byte(2);
                            for point in [start_mm, end_mm, center_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                            self.byte(u8::from(*clockwise));
                        }
                    }
                }
            }
            FeatureKind::SplineProfile { control_points_mm } => {
                self.byte(14);
                self.u64(control_points_mm.len() as u64);
                for point in control_points_mm {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            FeatureKind::Extrusion { profile, height } => {
                self.byte(2);
                self.u64(profile.0);
                self.bytes(height.source_token.as_bytes());
                self.u64(height.millimetres.to_bits());
            }
            FeatureKind::Pad(spec) => {
                self.byte(19);
                self.u64(spec.sketch.0);
                self.u64(spec.region.0);
                self.feature_direction(spec.direction);
                self.feature_extent(&spec.extent);
            }
            FeatureKind::SketchPocket(spec) => {
                self.byte(20);
                self.u64(spec.target.0);
                self.u64(spec.sketch.0);
                self.u64(spec.region.0);
                self.feature_direction(spec.direction);
                self.feature_extent(&spec.extent);
                self.body_subshape_reference(&spec.support);
            }
            FeatureKind::ThroughCut { target, profile } => {
                self.byte(3);
                self.u64(target.0);
                self.u64(profile.0);
            }
            FeatureKind::Pocket {
                target,
                profile,
                depth,
            } => {
                self.byte(10);
                self.u64(target.0);
                self.u64(profile.0);
                self.bytes(depth.source_token.as_bytes());
                self.u64(depth.millimetres.to_bits());
            }
            FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => {
                self.byte(8);
                self.byte(match operation {
                    BooleanOperation::Cut => 1,
                    BooleanOperation::Union => 2,
                    BooleanOperation::Intersect => 3,
                    BooleanOperation::Split => 4,
                });
                self.u64(target.0);
                self.u64(tool.0);
            }
            FeatureKind::PlanarOffset { profile, distance } => {
                self.byte(12);
                self.u64(profile.0);
                self.bytes(distance.source_token.as_bytes());
                self.u64(distance.millimetres.to_bits());
            }
            FeatureKind::Sweep { profile, path } => {
                self.byte(13);
                self.u64(profile.0);
                self.u64(path.0);
            }
            FeatureKind::Loft { sections } => {
                self.byte(15);
                self.u64(sections.len() as u64);
                for section in sections {
                    self.u64(section.profile.0);
                    self.u64(section.elevation_mm.to_bits());
                }
            }
            FeatureKind::Revolve {
                profile,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => {
                self.byte(4);
                self.u64(profile.0);
                for coordinate in axis_start_mm.iter().chain(axis_end_mm) {
                    self.u64(coordinate.to_bits());
                }
                self.u64(angle_degrees.to_bits());
            }
            FeatureKind::BottleProfileControl {
                profile,
                body_radius,
                body_height,
                shoulder_rise,
            } => {
                self.byte(6);
                self.u64(profile.0);
                for dimension in [body_radius, body_height, shoulder_rise] {
                    self.bytes(dimension.source_token.as_bytes());
                    self.u64(dimension.millimetres.to_bits());
                }
            }
            FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
            } => {
                self.byte(5);
                self.u64(target.0);
                self.u64(removed_faces.len() as u64);
                for role in removed_faces {
                    self.bytes(role.as_str().as_bytes());
                }
                self.bytes(thickness.source_token.as_bytes());
                self.u64(thickness.millimetres.to_bits());
            }
            FeatureKind::BottleEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => {
                self.byte(7);
                self.u64(target.0);
                self.u64(edges.len() as u64);
                for role in edges {
                    self.bytes(role.as_str().as_bytes());
                }
                self.byte(match kind {
                    BottleEdgeFinishKind::Fillet => 1,
                    BottleEdgeFinishKind::Chamfer => 2,
                });
                self.bytes(amount.source_token.as_bytes());
                self.u64(amount.millimetres.to_bits());
            }
            FeatureKind::TopologyShell {
                target,
                removed_faces,
                thickness,
            } => {
                self.byte(21);
                self.u64(target.0);
                self.u64(removed_faces.len() as u64);
                for reference in removed_faces {
                    self.topological_reference(reference);
                }
                self.bytes(thickness.source_token.as_bytes());
                self.u64(thickness.millimetres.to_bits());
            }
            FeatureKind::TopologyEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => {
                self.byte(22);
                self.u64(target.0);
                self.u64(edges.len() as u64);
                for reference in edges {
                    self.topological_reference(reference);
                }
                self.byte(match kind {
                    EdgeFinishKind::Fillet => 1,
                    EdgeFinishKind::Chamfer => 2,
                });
                self.bytes(amount.source_token.as_bytes());
                self.u64(amount.millimetres.to_bits());
            }
            FeatureKind::TopologyFaceOffset {
                target,
                face,
                distance,
            } => {
                self.byte(23);
                self.u64(target.0);
                self.topological_reference(face);
                self.bytes(distance.source_token.as_bytes());
                self.u64(distance.millimetres.to_bits());
            }
            FeatureKind::ImportedExactBody(spec) => {
                self.byte(16);
                self.bytes(spec.schema.as_bytes());
                self.u64(spec.import_id.0);
                self.bytes(&spec.source_sha256);
                self.u64(spec.source_byte_len);
                self.bytes(spec.result_fingerprint.as_bytes());
                self.u64(u64::from(spec.solid_count));
                if let Some(topology_counts) = spec.topology_counts {
                    self.byte(1);
                    for count in topology_counts {
                        self.u64(u64::from(count));
                    }
                }
                self.u64(spec.volume_mm3.to_bits());
                for coordinate in spec.bounds_mm.iter().flatten() {
                    self.u64(coordinate.to_bits());
                }
                self.bytes(spec.backend.as_bytes());
                self.bytes(spec.tolerance.as_bytes());
            }
            FeatureKind::MeshBody(spec) => {
                self.byte(9);
                self.bytes(spec.schema.as_bytes());
                self.u64(spec.vertices_mm.len() as u64);
                for vertex in &spec.vertices_mm {
                    for coordinate in vertex {
                        self.u64(coordinate.to_bits());
                    }
                }
                self.u64(spec.triangles.len() as u64);
                for triangle in &spec.triangles {
                    for index in triangle {
                        self.u64(u64::from(*index));
                    }
                }
                match &spec.authority {
                    MeshAuthority::Authored { provenance } => {
                        self.byte(1);
                        self.bytes(provenance.as_bytes());
                    }
                    MeshAuthority::ImportedStl { import_id } => {
                        self.byte(3);
                        self.u64(import_id.0);
                    }
                    MeshAuthority::ImportedSketchupScene { import_id } => {
                        self.byte(4);
                        self.u64(import_id.0);
                    }
                    MeshAuthority::ExactConversion(conversion) => {
                        self.byte(2);
                        self.u64(conversion.source_document_id.0);
                        self.u64(conversion.source_revision);
                        self.bytes(conversion.source_digest.as_bytes());
                        self.u64(conversion.source_definition_id.0);
                        self.u64(conversion.source_feature_id.0);
                        self.bytes(conversion.source_result_fingerprint.as_bytes());
                        self.bytes(conversion.source_evaluator.as_bytes());
                        self.bytes(conversion.source_backend.as_bytes());
                        self.bytes(conversion.source_tolerance.as_bytes());
                        self.bytes(conversion.tessellation_tolerance.as_bytes());
                        self.u64(conversion.destination_definition_id.0);
                        self.u64(conversion.destination_feature_id.0);
                        self.u64(conversion.unsupported_semantics.len() as u64);
                        for semantic in &conversion.unsupported_semantics {
                            self.bytes(semantic.as_bytes());
                        }
                        self.byte(match conversion.exact_reference_consequence {
                            ExactReferenceConversionConsequence::Lost => 1,
                        });
                    }
                }
            }
        }
    }

    fn feature(&mut self, feature: &Feature) {
        self.u64(feature.id.0);
        self.u64(feature.definition_id.0);
        self.bytes(feature.name.as_bytes());
        self.feature_kind(&feature.kind);
    }

    fn assembly_mate(&mut self, mate: &AssemblyMate) {
        self.u64(mate.id().0);
        for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
            self.u64(endpoint.occurrence_id().0);
            self.body_subshape_reference(endpoint.reference());
            match endpoint.health() {
                AssemblyReferenceHealth::Resolved => self.byte(1),
                AssemblyReferenceHealth::Ambiguous { candidate_count } => {
                    self.byte(2);
                    self.u64(u64::from(candidate_count));
                }
                AssemblyReferenceHealth::Lost => self.byte(3),
                AssemblyReferenceHealth::Broken => self.byte(4),
            }
        }
        match mate.kind() {
            AssemblyMateKind::CoincidentPlanar {
                offset_mm,
                reversed,
            } => {
                self.byte(1);
                self.u64(offset_mm.to_bits());
                self.byte(u8::from(reversed));
            }
            AssemblyMateKind::ConcentricAxial { reversed } => {
                self.byte(2);
                self.byte(u8::from(reversed));
            }
            AssemblyMateKind::Distance { distance_mm } => {
                self.byte(3);
                self.u64(distance_mm.to_bits());
            }
            AssemblyMateKind::Angle { angle_degrees } => {
                self.byte(4);
                self.u64(angle_degrees.to_bits());
            }
        }
    }

    fn assembly_joint(&mut self, joint: &AssemblyJoint) {
        self.bytes(joint.schema().as_bytes());
        self.u64(joint.id().0);
        self.u64(joint.parent_occurrence_id().0);
        self.u64(joint.child_occurrence_id().0);
        self.assembly_joint_kind(joint.kind());
    }

    fn assembly_joint_kind(&mut self, kind: AssemblyJointKind) {
        match kind {
            AssemblyJointKind::Fixed => self.byte(1),
            AssemblyJointKind::Revolute {
                axis,
                limits,
                position_degrees,
            } => {
                self.byte(2);
                self.assembly_joint_axis(axis);
                self.assembly_joint_limits(limits);
                self.u64(position_degrees.to_bits());
            }
            AssemblyJointKind::Prismatic {
                axis,
                limits,
                position_mm,
            } => {
                self.byte(3);
                self.assembly_joint_axis(axis);
                self.assembly_joint_limits(limits);
                self.u64(position_mm.to_bits());
            }
        }
    }

    fn assembly_joint_axis(&mut self, axis: crate::assembly_joint::AssemblyJointAxis) {
        for value in axis.direction_in_parent() {
            self.u64(value.to_bits());
        }
        for value in axis.pivot_in_parent_mm() {
            self.u64(value.to_bits());
        }
    }

    fn assembly_joint_limits(&mut self, limits: Option<AssemblyJointLimits>) {
        if let Some(limits) = limits {
            self.byte(1);
            self.u64(limits.min().to_bits());
            self.u64(limits.max().to_bits());
        } else {
            self.byte(0);
        }
    }

    fn assembly_motion_coupling(&mut self, coupling: &AssemblyMotionCoupling) {
        use crate::mechanical_coupling::{
            AssemblyMotionDirection, AssemblyTransmissionKind, GearMeshKind, ScrewHandedness,
        };

        self.bytes(coupling.schema().as_bytes());
        self.u64(coupling.id().0);
        self.u64(coupling.input_joint_id().0);
        self.u64(coupling.output_joint_id().0);
        self.u64(coupling.input_reference_position().to_bits());
        self.u64(coupling.output_reference_position().to_bits());
        match coupling.transmission() {
            AssemblyTransmissionKind::GearPair {
                input_teeth,
                output_teeth,
                mesh,
            } => {
                self.byte(1);
                self.u64(u64::from(input_teeth));
                self.u64(u64::from(output_teeth));
                self.byte(match mesh {
                    GearMeshKind::External => 1,
                    GearMeshKind::Internal => 2,
                });
            }
            AssemblyTransmissionKind::Belt {
                input_pitch_diameter_mm,
                output_pitch_diameter_mm,
                crossed,
            } => {
                self.byte(2);
                self.u64(input_pitch_diameter_mm.to_bits());
                self.u64(output_pitch_diameter_mm.to_bits());
                self.byte(u8::from(crossed));
            }
            AssemblyTransmissionKind::Chain {
                input_sprocket_teeth,
                output_sprocket_teeth,
            } => {
                self.byte(3);
                self.u64(u64::from(input_sprocket_teeth));
                self.u64(u64::from(output_sprocket_teeth));
            }
            AssemblyTransmissionKind::RackAndPinion {
                pinion_pitch_diameter_mm,
                direction,
            } => {
                self.byte(4);
                self.u64(pinion_pitch_diameter_mm.to_bits());
                self.byte(match direction {
                    AssemblyMotionDirection::Same => 1,
                    AssemblyMotionDirection::Opposite => 2,
                });
            }
            AssemblyTransmissionKind::LeadScrew {
                lead_mm_per_revolution,
                handedness,
            } => {
                self.byte(5);
                self.u64(lead_mm_per_revolution.to_bits());
                self.byte(match handedness {
                    ScrewHandedness::Right => 1,
                    ScrewHandedness::Left => 2,
                });
            }
        }
    }

    fn assembly_motion_study(&mut self, study: &AssemblyMotionStudy) {
        self.bytes(study.schema().as_bytes());
        self.u64(study.id().0);
        self.bytes(study.name().as_bytes());
        self.u64(study.drivers().len() as u64);
        for driver in study.drivers() {
            self.u64(driver.joint_id().0);
            self.u64(driver.position().to_bits());
        }
    }

    fn drawing_sheet(&mut self, sheet: &DrawingSheet) {
        self.bytes(sheet.schema().as_bytes());
        self.u64(sheet.id().0);
        self.bytes(sheet.name().as_bytes());
        match sheet.source() {
            DrawingSource::Definition(id) => {
                self.byte(1);
                self.u64(id.0);
            }
            DrawingSource::RigidAssembly { occurrence_ids } => {
                self.byte(2);
                self.u64(occurrence_ids.len() as u64);
                for id in occurrence_ids {
                    self.u64(id.0);
                }
            }
        }
    }

    fn occurrence(&mut self, occurrence: &Occurrence) {
        self.u64(occurrence.id.0);
        self.u64(occurrence.definition_id.0);
        self.bytes(occurrence.name.as_bytes());
        self.transform(occurrence.transform);
        self.optional_id(occurrence.parent.map(|id| id.0));
        self.optional_id(occurrence.tag.map(|id| id.0));
        self.byte(u8::from(occurrence.visible));
    }

    fn group(&mut self, group: &Group) {
        self.u64(group.id.0);
        self.bytes(group.name.as_bytes());
        self.transform(group.transform);
        self.optional_id(group.parent.map(|id| id.0));
    }

    fn local_group(&mut self, group: &LocalGroup) {
        self.u64(group.key.definition_id.0);
        self.u64(group.key.local_id.0);
        self.bytes(group.name.as_bytes());
        self.transform(group.transform);
        self.optional_id(group.parent.map(|id| id.0));
    }

    fn local_occurrence(&mut self, occurrence: &LocalOccurrence) {
        self.u64(occurrence.key.definition_id.0);
        self.u64(occurrence.key.local_id.0);
        self.u64(occurrence.definition_id.0);
        self.bytes(occurrence.name.as_bytes());
        self.transform(occurrence.transform);
        self.optional_id(occurrence.parent.map(|id| id.0));
        self.optional_id(occurrence.tag.map(|id| id.0));
        self.byte(u8::from(occurrence.visible));
    }

    fn authoritative_dependency(
        &mut self,
        product: &ProductModel,
        dependency: AuthoritativeDependency,
    ) {
        match dependency {
            AuthoritativeDependency::EvaluatorNode(id) => {
                self.byte(1);
                self.u64(id.0);
                if let Some(node) = product.evaluator_nodes.get(&id) {
                    self.byte(1);
                    self.node(node);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Override(id) => {
                self.byte(12);
                self.u64(id);
                if let Some(value) = product.overrides.get(&id) {
                    self.byte(1);
                    self.canonical_override(value);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::FeatureParameterBinding(target) => {
                self.byte(14);
                self.feature_parameter_target(&target);
                if let Some(binding) = product.feature_parameter_bindings.get(&target) {
                    self.byte(1);
                    self.feature_parameter_binding(binding);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Joint(id) => {
                self.byte(13);
                self.u64(id.0);
                if let Some(joint) = product.joints.get(&id) {
                    self.byte(1);
                    self.joint(joint);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Space(id) => {
                self.byte(19);
                self.u64(id.0);
                if let Some(space) = product.spaces.get(&id) {
                    self.byte(1);
                    self.space(space);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::ClearanceVolume(id) => {
                self.byte(20);
                self.u64(id.0);
                if let Some(clearance) = product.clearance_volumes.get(&id) {
                    self.byte(1);
                    self.clearance_volume(clearance);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::PersistentDimension(id) => {
                self.byte(15);
                self.u64(id.0);
                if let Some(dimension) = product.persistent_dimensions.get(&id) {
                    self.byte(1);
                    self.persistent_dimension(dimension);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Tag(id) => {
                self.byte(16);
                self.u64(id.0);
                if let Some(tag) = product.tags.get(&id) {
                    self.byte(1);
                    self.tag(tag);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::ClassificationDimension(id) => {
                self.byte(27);
                self.u64(id.0);
                if let Some(dimension) = product.classification_dimensions.get(&id) {
                    self.byte(1);
                    self.bytes(dimension.name.as_bytes());
                    self.u64(dimension.categories.len() as u64);
                    for category in dimension.categories.values() {
                        self.u64(category.id.0);
                        self.bytes(category.name.as_bytes());
                    }
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::OccurrenceClassification(occurrence_id, dimension_id) => {
                self.byte(28);
                self.u64(occurrence_id.0);
                self.u64(dimension_id.0);
                self.optional_id(
                    product
                        .classification_assignments
                        .get(&(occurrence_id, dimension_id))
                        .map(|id| id.0),
                );
            }
            AuthoritativeDependency::Collection(id) => {
                self.byte(17);
                self.u64(id.0);
                if let Some(collection) = product.collections.get(&id) {
                    self.byte(1);
                    self.collection(collection);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Import(id) => {
                self.byte(22);
                self.u64(id.0);
                if let Some(receipt) = product.import_receipts.get(&id) {
                    self.byte(1);
                    self.import_receipt(receipt);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Definition(id) => {
                self.byte(2);
                self.u64(id.0);
                if let Some(definition) = product.definitions.get(&id) {
                    self.byte(1);
                    self.definition(definition);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Feature(id) => {
                self.byte(3);
                self.u64(id.0);
                if let Some(feature) = product.features.get(&id) {
                    self.byte(1);
                    self.feature(feature);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::BodyFeatureSuppression(definition_id, body_id) => {
                self.byte(26);
                self.u64(definition_id.0);
                self.u64(body_id.0);
                if let Some(suppressed) = product
                    .body_feature_suppression
                    .get(&(definition_id, body_id))
                {
                    self.byte(1);
                    self.u64(suppressed.len() as u64);
                    for feature_id in suppressed {
                        self.u64(feature_id.0);
                    }
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Occurrence(id) => {
                self.byte(4);
                self.u64(id.0);
                if let Some(occurrence) = product.occurrences.get(&id) {
                    self.byte(1);
                    self.occurrence(occurrence);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::GroundedOccurrence(id) => {
                self.byte(23);
                self.u64(id.0);
                self.byte(u8::from(product.grounded_occurrences.contains(&id)));
            }
            AuthoritativeDependency::AssemblyMate(id) => {
                self.byte(24);
                self.u64(id.0);
                if let Some(mate) = product.assembly_mates.get(&id) {
                    self.byte(1);
                    self.assembly_mate(mate);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::AssemblyJoint(id) => {
                self.byte(34);
                self.u64(id.0);
                if let Some(joint) = product.assembly_joints.get(&id) {
                    self.byte(1);
                    self.assembly_joint(joint);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::AssemblyMotionCoupling(id) => {
                self.byte(36);
                self.u64(id.0);
                if let Some(coupling) = product.assembly_motion_couplings.get(&id) {
                    self.byte(1);
                    self.assembly_motion_coupling(coupling);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::AssemblyMotionStudy(id) => {
                self.byte(35);
                self.u64(id.0);
                if let Some(study) = product.assembly_motion_studies.get(&id) {
                    self.byte(1);
                    self.assembly_motion_study(study);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::DrawingSheet(id) => {
                self.byte(25);
                self.u64(id.0);
                if let Some(sheet) = product.drawing_sheets.get(&id) {
                    self.byte(1);
                    self.drawing_sheet(sheet);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Group(id) => {
                self.byte(5);
                self.u64(id.0);
                if let Some(group) = product.groups.get(&id) {
                    self.byte(1);
                    self.group(group);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::LocalGroup(key) => {
                self.byte(6);
                self.u64(key.definition_id.0);
                self.u64(key.local_id.0);
                if let Some(group) = product.local_groups.get(&key) {
                    self.byte(1);
                    self.local_group(group);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::LocalOccurrence(key) => {
                self.byte(7);
                self.u64(key.definition_id.0);
                self.u64(key.local_id.0);
                if let Some(occurrence) = product.local_occurrences.get(&key) {
                    self.byte(1);
                    self.local_occurrence(occurrence);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::DefinitionUsers(id) => {
                self.byte(8);
                self.u64(id.0);
                let world_users = product
                    .occurrences
                    .values()
                    .filter(|occurrence| occurrence.definition_id == id)
                    .map(|occurrence| occurrence.id)
                    .collect::<Vec<_>>();
                self.u64(world_users.len() as u64);
                for occurrence_id in world_users {
                    self.u64(occurrence_id.0);
                }
                let local_users = product
                    .local_occurrences
                    .values()
                    .filter(|occurrence| occurrence.definition_id == id)
                    .map(|occurrence| occurrence.key)
                    .collect::<Vec<_>>();
                self.u64(local_users.len() as u64);
                for key in local_users {
                    self.u64(key.definition_id.0);
                    self.u64(key.local_id.0);
                }
            }
            AuthoritativeDependency::FeatureUsers(id) => {
                self.byte(9);
                self.u64(id.0);
                let users = product
                    .features
                    .values()
                    .filter_map(|feature| match feature.kind {
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::Offset { base, .. },
                            ..
                        }) if base == id => Some(feature.id),
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::PlanarFace { ref reference, .. },
                            ..
                        }) if reference.producer_feature_id == id => Some(feature.id),
                        FeatureKind::Sketch(ref spec) if spec.workplane == id => Some(feature.id),
                        FeatureKind::Extrusion { profile, .. }
                        | FeatureKind::PlanarOffset { profile, .. }
                            if profile == id =>
                        {
                            Some(feature.id)
                        }
                        FeatureKind::ThroughCut { target, profile }
                            if target == id || profile == id =>
                        {
                            Some(feature.id)
                        }
                        FeatureKind::Sweep { profile, path } if profile == id || path == id => {
                            Some(feature.id)
                        }
                        FeatureKind::Loft { ref sections }
                            if sections.iter().any(|section| section.profile == id) =>
                        {
                            Some(feature.id)
                        }
                        FeatureKind::Boolean { target, tool, .. } if target == id || tool == id => {
                            Some(feature.id)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                self.u64(users.len() as u64);
                for feature_id in users {
                    self.u64(feature_id.0);
                }
            }
            AuthoritativeDependency::FeatureParameterBindings(id) => {
                self.byte(21);
                self.u64(id.0);
                let bindings = product
                    .feature_parameter_bindings
                    .values()
                    .filter(|binding| binding.target.feature_id == id)
                    .collect::<Vec<_>>();
                self.u64(bindings.len() as u64);
                for binding in bindings {
                    self.feature_parameter_binding(binding);
                }
            }
            AuthoritativeDependency::GroupChildren(id) => {
                self.byte(10);
                self.u64(id.0);
                let group_children = product
                    .groups
                    .values()
                    .filter(|group| group.parent == Some(id))
                    .map(|group| group.id)
                    .collect::<Vec<_>>();
                self.u64(group_children.len() as u64);
                for group_id in group_children {
                    self.u64(group_id.0);
                }
                let occurrence_children = product
                    .occurrences
                    .values()
                    .filter(|occurrence| occurrence.parent == Some(id))
                    .map(|occurrence| occurrence.id)
                    .collect::<Vec<_>>();
                self.u64(occurrence_children.len() as u64);
                for occurrence_id in occurrence_children {
                    self.u64(occurrence_id.0);
                }
            }
            AuthoritativeDependency::GroupSubtree(root) => {
                self.byte(11);
                self.u64(root.0);
                let descendants = product
                    .groups
                    .keys()
                    .cloned()
                    .filter(|id| group_is_descendant(product, root, *id))
                    .collect::<BTreeSet<_>>();
                self.u64(descendants.len() as u64);
                for id in &descendants {
                    self.group(&product.groups[id]);
                }
                let occurrences = product
                    .occurrences
                    .values()
                    .filter(|occurrence| {
                        occurrence
                            .parent
                            .is_some_and(|parent| descendants.contains(&parent))
                    })
                    .collect::<Vec<_>>();
                self.u64(occurrences.len() as u64);
                for occurrence in occurrences {
                    self.occurrence(occurrence);
                }
            }
            AuthoritativeDependency::OccurrenceCollections(id) => {
                self.byte(18);
                self.u64(id.0);
                self.byte(u8::from(product.grounded_occurrences.contains(&id)));
                let collections = product
                    .collections
                    .values()
                    .filter(|collection| collection.occurrence_ids.contains(&id))
                    .collect::<Vec<_>>();
                self.u64(collections.len() as u64);
                for collection in collections {
                    self.collection(collection);
                }
                let mates = product
                    .assembly_mates
                    .values()
                    .filter(|mate| {
                        mate.endpoint_a().occurrence_id() == id
                            || mate.endpoint_b().occurrence_id() == id
                    })
                    .collect::<Vec<_>>();
                self.u64(mates.len() as u64);
                for mate in mates {
                    self.assembly_mate(mate);
                }
            }
        }
    }

    fn optional_id(&mut self, id: Option<u64>) {
        match id {
            Some(id) => {
                self.byte(1);
                self.u64(id);
            }
            None => self.byte(0),
        }
    }

    fn command(&mut self, command: &CanonicalCommand) {
        match command {
            CanonicalCommand::CreateEvaluatorNode {
                id,
                name,
                dimension,
                dependencies,
            } => {
                self.byte(1);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
                self.u64(dependencies.len() as u64);
                for dependency in dependencies {
                    self.u64(dependency.0);
                }
            }
            CanonicalCommand::SetEvaluatorDimension { id, dimension } => {
                self.byte(2);
                self.u64(id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::RenameEvaluatorNode { id, name } => {
                self.byte(3);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::RecordImport(receipt) => {
                self.byte(50);
                self.import_receipt(receipt);
            }
            CanonicalCommand::CreateDefinition { id, name } => {
                self.byte(10);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::DeleteDefinition { id } => {
                self.byte(11);
                self.u64(id.0);
            }
            CanonicalCommand::RenameDefinition { id, name } => {
                self.byte(12);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::CreateBody {
                definition_id,
                id,
                name,
                visible,
            } => {
                self.byte(60);
                self.u64(definition_id.0);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::DeleteBody { definition_id, id } => {
                self.byte(61);
                self.u64(definition_id.0);
                self.u64(id.0);
            }
            CanonicalCommand::RenameBody {
                definition_id,
                id,
                name,
            } => {
                self.byte(62);
                self.u64(definition_id.0);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::SetActiveBody { definition_id, id } => {
                self.byte(63);
                self.u64(definition_id.0);
                self.u64(id.0);
            }
            CanonicalCommand::SetBodyVisibility {
                definition_id,
                id,
                visible,
            } => {
                self.byte(64);
                self.u64(definition_id.0);
                self.u64(id.0);
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::ConsumeBody {
                definition_id,
                id,
                by_feature_id,
            } => {
                self.byte(66);
                self.u64(definition_id.0);
                self.u64(id.0);
                self.u64(by_feature_id.0);
            }
            CanonicalCommand::SetFeatureBodyOwnership { id, ownership } => {
                self.byte(65);
                self.u64(id.0);
                self.u64(ownership.input_body_ids.len() as u64);
                for body_id in &ownership.input_body_ids {
                    self.u64(body_id.0);
                }
                self.optional_id(ownership.output_body_id.map(|body_id| body_id.0));
            }
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id,
                body_id,
                suppressed_feature_ids,
            } => {
                self.byte(68);
                self.u64(definition_id.0);
                self.u64(body_id.0);
                self.u64(suppressed_feature_ids.len() as u64);
                for feature_id in suppressed_feature_ids {
                    self.u64(feature_id.0);
                }
            }
            CanonicalCommand::CreateFeature {
                id,
                definition_id,
                name,
                kind,
            } => {
                self.byte(13);
                self.u64(id.0);
                self.u64(definition_id.0);
                self.bytes(name.as_bytes());
                self.feature_kind(kind);
            }
            CanonicalCommand::DeleteFeature { id } => {
                self.byte(14);
                self.u64(id.0);
            }
            CanonicalCommand::SetFeatureDimension { id, dimension } => {
                self.byte(15);
                self.u64(id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::SetSketchConstraintDimension {
                id,
                constraint_id,
                dimension,
            } => {
                self.byte(67);
                self.u64(id.0);
                self.u64(constraint_id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::TranslateProfile { id, delta_mm } => {
                self.byte(71);
                self.u64(id.0);
                self.u64(delta_mm[0].to_bits());
                self.u64(delta_mm[1].to_bits());
            }
            CanonicalCommand::SetBottleControlDimension {
                id,
                control,
                dimension,
            } => {
                self.byte(31);
                self.u64(id.0);
                self.byte(match control {
                    BottleControlDimension::BodyRadius => 1,
                    BottleControlDimension::BodyHeight => 2,
                    BottleControlDimension::ShoulderRise => 3,
                });
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::SetBottleEdgeFinishKind { id, kind } => {
                self.byte(32);
                self.u64(id.0);
                self.byte(match kind {
                    BottleEdgeFinishKind::Fillet => 1,
                    BottleEdgeFinishKind::Chamfer => 2,
                });
            }
            CanonicalCommand::SetProfilePoints { id, points_mm } => {
                self.byte(27);
                self.u64(id.0);
                self.u64(points_mm.len() as u64);
                for point in points_mm {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            CanonicalCommand::CreateOccurrence {
                id,
                definition_id,
                name,
                transform,
                parent,
                tag,
                visible,
            } => {
                self.byte(16);
                self.u64(id.0);
                self.u64(definition_id.0);
                self.bytes(name.as_bytes());
                self.transform(*transform);
                self.optional_id(parent.map(|id| id.0));
                self.optional_id(tag.map(|id| id.0));
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::DeleteOccurrence { id } => {
                self.byte(17);
                self.u64(id.0);
            }
            CanonicalCommand::SetOccurrenceTransform { id, transform } => {
                self.byte(18);
                self.u64(id.0);
                self.transform(*transform);
            }
            CanonicalCommand::RenameEntity { id, name } => {
                self.byte(69);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::GuardAssemblyRecompute {
                source_revision,
                source_digest,
            } => {
                self.byte(56);
                self.u64(*source_revision);
                self.bytes(source_digest.as_bytes());
            }
            CanonicalCommand::ApplyAssemblySolve {
                source_revision,
                source_digest,
                transforms,
            } => {
                self.byte(54);
                self.u64(*source_revision);
                self.bytes(source_digest.as_bytes());
                self.u64(transforms.len() as u64);
                for (id, transform) in transforms {
                    self.u64(id.0);
                    self.transform(*transform);
                }
            }
            CanonicalCommand::SetOccurrenceGrounded { id, grounded } => {
                self.byte(50);
                self.u64(id.0);
                self.byte(u8::from(*grounded));
            }
            CanonicalCommand::CreateAssemblyMate(mate) => {
                self.byte(51);
                self.assembly_mate(mate);
            }
            CanonicalCommand::RebindAssemblyMate(mate) => {
                self.byte(55);
                self.assembly_mate(mate);
            }
            CanonicalCommand::SetAssemblyMateKind { id, kind } => {
                self.byte(52);
                self.u64(id.0);
                match kind {
                    AssemblyMateKind::CoincidentPlanar {
                        offset_mm,
                        reversed,
                    } => {
                        self.byte(1);
                        self.u64(offset_mm.to_bits());
                        self.byte(u8::from(*reversed));
                    }
                    AssemblyMateKind::ConcentricAxial { reversed } => {
                        self.byte(2);
                        self.byte(u8::from(*reversed));
                    }
                    AssemblyMateKind::Distance { distance_mm } => {
                        self.byte(3);
                        self.u64(distance_mm.to_bits());
                    }
                    AssemblyMateKind::Angle { angle_degrees } => {
                        self.byte(4);
                        self.u64(angle_degrees.to_bits());
                    }
                }
            }
            CanonicalCommand::DeleteAssemblyMate { id } => {
                self.byte(53);
                self.u64(id.0);
            }
            CanonicalCommand::CreateAssemblyJoint(joint) => {
                self.byte(74);
                self.assembly_joint(joint);
            }
            CanonicalCommand::SetAssemblyJointKind { id, kind } => {
                self.byte(75);
                self.u64(id.0);
                self.assembly_joint_kind(*kind);
            }
            CanonicalCommand::SetAssemblyJointPosition { id, position } => {
                self.byte(76);
                self.u64(id.0);
                self.u64(position.to_bits());
            }
            CanonicalCommand::SetAssemblyJointLimits { id, limits } => {
                self.byte(77);
                self.u64(id.0);
                self.assembly_joint_limits(*limits);
            }
            CanonicalCommand::DeleteAssemblyJoint { id } => {
                self.byte(78);
                self.u64(id.0);
            }
            CanonicalCommand::CreateAssemblyMotionCoupling(coupling) => {
                self.byte(82);
                self.assembly_motion_coupling(coupling);
            }
            CanonicalCommand::UpdateAssemblyMotionCoupling(coupling) => {
                self.byte(83);
                self.assembly_motion_coupling(coupling);
            }
            CanonicalCommand::DeleteAssemblyMotionCoupling { id } => {
                self.byte(84);
                self.u64(id.0);
            }
            CanonicalCommand::CreateAssemblyMotionStudy(study) => {
                self.byte(79);
                self.assembly_motion_study(study);
            }
            CanonicalCommand::UpdateAssemblyMotionStudy(study) => {
                self.byte(80);
                self.assembly_motion_study(study);
            }
            CanonicalCommand::DeleteAssemblyMotionStudy { id } => {
                self.byte(81);
                self.u64(id.0);
            }
            CanonicalCommand::CreateDrawingSheet(sheet) => {
                self.byte(57);
                self.drawing_sheet(sheet);
            }
            CanonicalCommand::UpdateDrawingSheet(sheet) => {
                self.byte(58);
                self.drawing_sheet(sheet);
            }
            CanonicalCommand::DeleteDrawingSheet { id } => {
                self.byte(59);
                self.u64(id.0);
            }
            CanonicalCommand::SetOccurrenceVisibility { id, visible } => {
                self.byte(19);
                self.u64(id.0);
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::SetOccurrenceTag { id, tag } => {
                self.byte(41);
                self.u64(id.0);
                self.optional_id(tag.map(|id| id.0));
            }
            CanonicalCommand::RepointOccurrence { id, definition_id } => {
                self.byte(20);
                self.u64(id.0);
                self.u64(definition_id.0);
            }
            CanonicalCommand::SetOccurrenceParent { id, parent } => {
                self.byte(21);
                self.u64(id.0);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::CreateGroup {
                id,
                name,
                transform,
                parent,
            } => {
                self.byte(22);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.transform(*transform);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::DeleteGroup { id } => {
                self.byte(23);
                self.u64(id.0);
            }
            CanonicalCommand::SetGroupTransform { id, transform } => {
                self.byte(24);
                self.u64(id.0);
                self.transform(*transform);
            }
            CanonicalCommand::SetGroupParent { id, parent } => {
                self.byte(25);
                self.u64(id.0);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                self.byte(26);
                self.u64(plan.occurrence_id.0);
                self.u64(plan.source_definition_id.0);
                self.u64(plan.new_definition_id.0);
                self.bytes(plan.new_definition_name.as_bytes());
                self.u64(plan.feature_id_map.len() as u64);
                for (source_id, new_id) in &plan.feature_id_map {
                    self.u64(source_id.0);
                    self.u64(new_id.0);
                }
            }
            CanonicalCommand::ConvertGroupToComponent(plan) => {
                self.byte(28);
                self.u64(plan.group_id.0);
                self.u64(plan.new_definition_id.0);
                self.u64(plan.new_occurrence_id.0);
                self.bytes(plan.component_name.as_bytes());
            }
            CanonicalCommand::ApplySolidTool(plan) => {
                self.byte(49);
                self.byte(match plan.operation {
                    BooleanOperation::Cut => 1,
                    BooleanOperation::Union => 2,
                    BooleanOperation::Intersect => 3,
                    BooleanOperation::Split => 4,
                });
                self.u64(plan.target_occurrence_id.0);
                self.u64(plan.target_feature_id.0);
                self.u64(plan.tool_occurrence_id.0);
                self.u64(plan.tool_feature_id.0);
                self.u64(plan.result_definition_id.0);
                for id in plan.result_feature_ids {
                    self.u64(id.0);
                }
                self.bytes(plan.result_definition_name.as_bytes());
                self.bytes(plan.result_feature_name.as_bytes());
                self.byte(u8::from(plan.keep_tool));
            }
            CanonicalCommand::CreateExpressionNode {
                id,
                name,
                expression,
            } => {
                self.byte(4);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(expression.as_bytes());
            }
            CanonicalCommand::CreateRuleNode {
                id,
                name,
                expression,
                input_ports,
                output_ports,
                outputs,
                override_parameters,
            } => {
                self.byte(5);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(expression.as_bytes());
                self.ports(input_ports);
                self.ports(output_ports);
                self.rule_outputs(outputs);
                self.u64(override_parameters.len() as u64);
                for parameter in override_parameters {
                    self.bytes(parameter.name().as_bytes());
                    self.byte(match parameter.merge_policy() {
                        OverrideMergePolicy::Replace => 1,
                    });
                }
            }
            CanonicalCommand::SetNodeExpression { id, expression } => {
                self.byte(6);
                self.u64(id.0);
                self.bytes(expression.as_bytes());
            }
            CanonicalCommand::SetRuleOutputs { id, outputs } => {
                self.byte(7);
                self.u64(id.0);
                self.rule_outputs(outputs);
            }
            CanonicalCommand::UpsertOverride(value) => {
                self.byte(8);
                self.canonical_override(value);
            }
            CanonicalCommand::DeleteOverride { id } => {
                self.byte(9);
                self.u64(*id);
            }
            CanonicalCommand::UpsertFeatureParameterBinding(binding) => {
                self.byte(33);
                self.feature_parameter_binding(binding);
            }
            CanonicalCommand::DeleteFeatureParameterBinding { target } => {
                self.byte(34);
                self.feature_parameter_target(target);
            }
            CanonicalCommand::RecomputeFeatureParameters { identity } => {
                self.byte(35);
                self.bytes(identity.evaluator.as_bytes());
                self.bytes(identity.schema.as_bytes());
                self.bytes(identity.tolerance.as_bytes());
                match &identity.backend {
                    Some(backend) => {
                        self.byte(1);
                        self.bytes(backend.as_bytes());
                    }
                    None => self.byte(0),
                }
            }
            CanonicalCommand::UpsertJoint(joint) => {
                self.byte(29);
                self.joint(joint);
            }
            CanonicalCommand::DeleteJoint { id } => {
                self.byte(30);
                self.u64(id.0);
            }
            CanonicalCommand::UpsertSpace(space) => {
                self.byte(45);
                self.space(space);
            }
            CanonicalCommand::DeleteSpace { id } => {
                self.byte(46);
                self.u64(id.0);
            }
            CanonicalCommand::UpsertClearanceVolume(clearance) => {
                self.byte(47);
                self.clearance_volume(clearance);
            }
            CanonicalCommand::DeleteClearanceVolume { id } => {
                self.byte(48);
                self.u64(id.0);
            }
            CanonicalCommand::UpsertPersistentDimension(dimension) => {
                self.byte(36);
                self.persistent_dimension(dimension);
            }
            CanonicalCommand::DeletePersistentDimension { id } => {
                self.byte(37);
                self.u64(id.0);
            }
            CanonicalCommand::CreateTag { id, name, visible } => {
                self.byte(38);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::DeleteTag { id } => {
                self.byte(39);
                self.u64(id.0);
            }
            CanonicalCommand::SetTagVisibility { id, visible } => {
                self.byte(40);
                self.u64(id.0);
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::SetTagName { id, name } => {
                self.byte(70);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::UpsertClassificationDimension {
                id,
                name,
                categories,
            } => {
                self.byte(72);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.u64(categories.len() as u64);
                for (category_id, category_name) in categories {
                    self.u64(category_id.0);
                    self.bytes(category_name.as_bytes());
                }
            }
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id,
                dimension_id,
                category_id,
            } => {
                self.byte(73);
                self.u64(occurrence_id.0);
                self.u64(dimension_id.0);
                self.optional_id(category_id.map(|id| id.0));
            }
            CanonicalCommand::CreateCollection { id, name } => {
                self.byte(42);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::DeleteCollection { id } => {
                self.byte(43);
                self.u64(id.0);
            }
            CanonicalCommand::SetCollectionOccurrences { id, occurrence_ids } => {
                self.byte(44);
                self.u64(id.0);
                self.u64(occurrence_ids.len() as u64);
                for occurrence_id in occurrence_ids {
                    self.u64(occurrence_id.0);
                }
            }
        }
    }

    fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}

#[cfg(test)]
mod parameter_contract_tests {
    use super::*;
    use crate::sketch::{
        FeatureDirection, SketchConstraint, SketchEntityId, SketchPointRef, SketchRegionId,
    };

    fn dimension(value: f64) -> Dimension {
        Dimension::new(value.to_string(), value).unwrap()
    }

    fn assert_descriptors_have_read_write_accessors(kind: FeatureKind) {
        validate_feature_kind(&kind).unwrap();
        let descriptors = kind.parameter_descriptors();
        assert!(!descriptors.is_empty());
        for descriptor in descriptors {
            let value = feature_kind_parameter_value(&kind, descriptor.path().as_str())
                .unwrap_or_else(|| panic!("missing reader for {}", descriptor.path().as_str()));
            let target = FeatureParameterTarget::new(
                FeatureId(99),
                descriptor.path().as_str(),
                descriptor.value_type(),
            )
            .unwrap();
            let mut updated = kind.clone();
            assert!(
                set_feature_kind_parameter(&mut updated, &target, &dimension(value)).unwrap(),
                "missing writer for {}",
                descriptor.path().as_str()
            );
            validate_feature_kind(&updated).unwrap_or_else(|error| {
                panic!(
                    "writer for {} produced invalid feature: {error}",
                    descriptor.path().as_str()
                )
            });
            assert_eq!(
                feature_kind_parameter_value(&updated, descriptor.path().as_str()),
                Some(value)
            );
        }
    }

    #[test]
    fn every_structural_descriptor_has_a_valid_read_write_round_trip() {
        let sketch = FeatureKind::Sketch(SketchSpec {
            workplane: FeatureId(1),
            entities: vec![SketchEntity::Circle {
                id: SketchEntityId(1),
                center_mm: [3.0, 4.0],
                radius_mm: 2.0,
            }],
            constraints: vec![
                SketchConstraint {
                    id: SketchConstraintId(1),
                    kind: SketchConstraintKind::Radius {
                        entity: SketchEntityId(1),
                        value: dimension(2.0),
                    },
                },
                SketchConstraint {
                    id: SketchConstraintId(2),
                    kind: SketchConstraintKind::FixedPoint {
                        point: SketchPointRef {
                            entity: SketchEntityId(1),
                            point: SketchPointKind::Center,
                        },
                        position_mm: [3.0, 4.0],
                    },
                },
            ],
        });
        let kinds = vec![
            FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Offset {
                    base: FeatureId(1),
                    distance: dimension(8.0),
                },
                frame: WorkplaneFrame::principal(PrincipalPlane::Xy),
            }),
            sketch,
            FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
            },
            FeatureKind::SegmentProfile {
                segments: vec![ProfileSegment::Line {
                    start_mm: [0.0, 0.0],
                    end_mm: [4.0, 2.0],
                }],
                closed: false,
            },
            FeatureKind::SplineProfile {
                control_points_mm: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]],
            },
            FeatureKind::Extrusion {
                profile: FeatureId(1),
                height: dimension(12.0),
            },
            FeatureKind::Pad(PadSpec {
                sketch: FeatureId(1),
                region: SketchRegionId(1),
                direction: FeatureDirection::AlongNormal,
                extent: FeatureExtent::Bidirectional {
                    along: FeatureExtentEnd::Blind(dimension(6.0)),
                    opposite: FeatureExtentEnd::Blind(dimension(3.0)),
                },
            }),
            FeatureKind::BottleProfileControl {
                profile: FeatureId(1),
                body_radius: dimension(20.0),
                body_height: dimension(50.0),
                shoulder_rise: dimension(8.0),
            },
            FeatureKind::Revolve {
                profile: FeatureId(1),
                axis_start_mm: [0.0, 0.0],
                axis_end_mm: [0.0, 1.0],
                angle_degrees: 180.0,
            },
            FeatureKind::Pocket {
                target: FeatureId(2),
                profile: FeatureId(1),
                depth: dimension(5.0),
            },
            FeatureKind::PlanarOffset {
                profile: FeatureId(1),
                distance: dimension(2.0),
            },
            FeatureKind::Loft {
                sections: vec![
                    LoftSection {
                        profile: FeatureId(1),
                        elevation_mm: 0.0,
                    },
                    LoftSection {
                        profile: FeatureId(2),
                        elevation_mm: 10.0,
                    },
                ],
            },
        ];
        for kind in kinds {
            assert_descriptors_have_read_write_accessors(kind);
        }
    }
}
