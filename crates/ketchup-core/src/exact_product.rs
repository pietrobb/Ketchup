#![forbid(unsafe_code)]

#[cfg(feature = "named-product-fixtures")]
use crate::beam_m5::{BeamExactPiecePackage, BeamExactResultKey};
use crate::document::{
    BodyId, BooleanOperation, CanonicalCommand, CommandBatch, DefinitionId, DocumentId,
    EdgeFinishKind, ExactReferenceConversionConsequence, ExactToMeshConversion,
    FeatureDependencyGraph, FeatureId, FeatureKind, InstancePath, MESH_BODY_SCHEMA_V1,
    MeshAuthority, MeshBodySpec, ProfileSegment, Snapshot, Transform,
};
use crate::exact_brep_graph::{ExactBRepGraph, MAX_EXACT_BREP_LOFT_CONTROL_POINTS};
use crate::graph::DerivedIdentity;
use crate::import::StepImportMesh;
use crate::sketch::{
    FeatureDirection, SolvedSketchRegion, SolvedSketchRegionEdge, SolvedSketchRegionProfile,
    WorkplaneSpec, WorkplaneSupport, WorkplaneSupportHealth,
};
use crate::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceResolution,
    publish_generated_topological_references, publish_imported_topological_references,
    resolve_topological_reference as resolve_role_neutral_topological_reference,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

pub const EXACT_PRODUCT_SCHEMA_V1: &str = "ketchup.exact-product.v1";
pub const EXACT_BREP_GRAPH_EVALUATOR_V1: &str = "ketchup.exact-brep-graph-evaluator.v1";
pub const EXACT_RECTANGLE_EVALUATOR_V1: &str = "ketchup.exact-rectangle-evaluator.v1";
pub const EXACT_CIRCLE_EVALUATOR_V1: &str = "ketchup.exact-circle-evaluator.v1";
pub const EXACT_ARC_PROFILE_EVALUATOR_V1: &str = "ketchup.exact-arc-profile-evaluator.v1";
pub const EXACT_LINEAR_PROFILE_EVALUATOR_V1: &str = "ketchup.exact-linear-profile-evaluator.v1";
pub const EXACT_THROUGH_CUT_EVALUATOR_V1: &str = "ketchup.exact-through-cut-evaluator.v1";
pub const EXACT_CIRCULAR_CUT_EVALUATOR_V1: &str = "ketchup.exact-circular-cut-evaluator.v1";
pub const EXACT_POCKET_EVALUATOR_V1: &str = "ketchup.exact-pocket-evaluator.v1";
pub const EXACT_BOOLEAN_UNION_EVALUATOR_V1: &str = "ketchup.exact-boolean-union-evaluator.v1";
pub const EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1: &str =
    "ketchup.exact-boolean-intersect-evaluator.v1";
pub const EXACT_BOOLEAN_SPLIT_EVALUATOR_V1: &str = "ketchup.exact-boolean-split-evaluator.v1";
pub const EXACT_PLANAR_OFFSET_SCHEMA_V1: &str = "ketchup.exact-planar-offset.v1";
pub const EXACT_PLANAR_OFFSET_EVALUATOR_V1: &str = "ketchup.exact-planar-offset-evaluator.v1";
pub const EXACT_MIN_LENGTH_MM: f64 = 0.01;
pub const EXACT_SWEEP_SCHEMA_V1: &str = "ketchup.exact-sweep.v1";
pub const EXACT_SWEEP_EVALUATOR_V1: &str = "ketchup.exact-sweep-evaluator.v1";
pub const EXACT_LOFT_SCHEMA_V1: &str = "ketchup.exact-loft.v1";
pub const EXACT_LOFT_EVALUATOR_V1: &str = "ketchup.exact-loft-evaluator.v1";
pub const EXACT_BOX_SHELL_EVALUATOR_V1: &str = "ketchup.exact-box-shell-evaluator.v1";
pub const EXACT_BOX_FINISH_EVALUATOR_V1: &str = "ketchup.exact-box-finish-evaluator.v1";
pub const BODY_SUBSHAPE_REF_SCHEMA_V1: &str = "ketchup.body-subshape-ref.v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExactFaceRole {
    Top,
    Bottom,
    East,
    West,
    CircleSide,
    ArcSide,
    LinearSide,
    CutCircle,
    CutLinear,
    CutArc,
    CutWest,
    CutEast,
    CutSouth,
    CutNorth,
    PocketFloor,
    PocketWest,
    PocketEast,
    PocketSouth,
    PocketNorth,
    RevolveBottom,
    RevolveBody,
    RevolveShoulder,
    RevolveNeck,
    RevolveMouth,
    RevolveSide0,
    RevolveSide1,
    RevolveStart,
    RevolveEnd,
    ShellOuterBottom,
    ShellOuterBody,
    ShellOuterShoulder,
    ShellOuterNeck,
    ShellRim,
    ShellInnerBottom,
    ShellInnerBody,
    ShellInnerShoulder,
    ShellInnerNeck,
    BoxShellOuterBottom,
    BoxShellOuterEast,
    BoxShellRim,
    PlanarOffsetFace,
    SweepStart,
    SweepEnd,
    SweepSide0,
    SweepSide1,
    SweepSide2,
    SweepSide3,
    LoftStart,
    LoftEnd,
    LoftSide,
}

const EXTRUSION_FACE_ROLES: [ExactFaceRole; 3] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
];
const CIRCLE_EXTRUSION_FACE_ROLES: [ExactFaceRole; 3] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::CircleSide,
];
const ARC_EXTRUSION_FACE_ROLES: [ExactFaceRole; 3] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::ArcSide,
];
const LINEAR_EXTRUSION_FACE_ROLES: [ExactFaceRole; 3] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::LinearSide,
];
const CIRCULAR_CUT_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::CutCircle,
];
const WEST_CIRCULAR_CUT_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::West,
    ExactFaceRole::CutCircle,
];
const CIRCULAR_POCKET_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::Top,
    ExactFaceRole::East,
    ExactFaceRole::CutCircle,
    ExactFaceRole::PocketFloor,
];
const WEST_CIRCULAR_POCKET_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::Top,
    ExactFaceRole::West,
    ExactFaceRole::CutCircle,
    ExactFaceRole::PocketFloor,
];
const POLYGON_CUT_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::CutLinear,
];
const WEST_POLYGON_CUT_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::West,
    ExactFaceRole::CutLinear,
];
const ARC_POLYGON_CUT_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::CutArc,
];
const POLYGON_POCKET_FACE_ROLES: [ExactFaceRole; 5] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::CutLinear,
    ExactFaceRole::PocketFloor,
];
const ARC_POLYGON_POCKET_FACE_ROLES: [ExactFaceRole; 5] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::CutArc,
    ExactFaceRole::PocketFloor,
];
const THROUGH_CUT_FACE_ROLES: [ExactFaceRole; 7] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::CutWest,
    ExactFaceRole::CutEast,
    ExactFaceRole::CutSouth,
    ExactFaceRole::CutNorth,
];
const BOX_SHELL_FACE_ROLES: [ExactFaceRole; 3] = [
    ExactFaceRole::BoxShellOuterBottom,
    ExactFaceRole::BoxShellOuterEast,
    ExactFaceRole::BoxShellRim,
];
const POCKET_FACE_ROLES: [ExactFaceRole; 8] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::PocketFloor,
    ExactFaceRole::PocketWest,
    ExactFaceRole::PocketEast,
    ExactFaceRole::PocketSouth,
    ExactFaceRole::PocketNorth,
];

impl ExactFaceRole {
    #[must_use]
    pub const fn semantic_role(self) -> &'static str {
        match self {
            Self::Top => "extrusion.top",
            Self::Bottom => "extrusion.bottom",
            Self::East => "extrusion.side(profile_edge=east)",
            Self::West => "extrusion.side(profile_edge=west)",
            Self::CircleSide => "extrusion.side(profile_edge=circle)",
            Self::ArcSide => "extrusion.side(profile_edge=arc.0)",
            Self::LinearSide => "extrusion.side(profile_edge=line.0)",
            Self::CutCircle => "through_cut.wall.circle",
            Self::CutLinear => "through_cut.wall.line.0",
            Self::CutArc => "through_cut.wall.arc.0",
            Self::CutWest => "through_cut.wall.west",
            Self::CutEast => "through_cut.wall.east",
            Self::CutSouth => "through_cut.wall.south",
            Self::CutNorth => "through_cut.wall.north",
            Self::PocketFloor => "pocket.floor",
            Self::PocketWest => "pocket.wall.west",
            Self::PocketEast => "pocket.wall.east",
            Self::PocketSouth => "pocket.wall.south",
            Self::PocketNorth => "pocket.wall.north",
            Self::RevolveBottom => "revolve.bottom",
            Self::RevolveBody => "revolve.body",
            Self::RevolveShoulder => "revolve.shoulder",
            Self::RevolveNeck => "revolve.neck",
            Self::RevolveMouth => "revolve.mouth",
            Self::RevolveSide0 => "revolve.side.0",
            Self::RevolveSide1 => "revolve.side.1",
            Self::RevolveStart => "revolve.start",
            Self::RevolveEnd => "revolve.end",
            Self::ShellOuterBottom => "shell.outer.bottom",
            Self::ShellOuterBody => "shell.outer.body",
            Self::ShellOuterShoulder => "shell.outer.shoulder",
            Self::ShellOuterNeck => "shell.outer.neck",
            Self::ShellRim => "shell.rim",
            Self::ShellInnerBottom => "shell.inner.bottom",
            Self::ShellInnerBody => "shell.inner.body",
            Self::ShellInnerShoulder => "shell.inner.shoulder",
            Self::ShellInnerNeck => "shell.inner.neck",
            Self::BoxShellOuterBottom => "shell.box.outer.bottom",
            Self::BoxShellOuterEast => "shell.box.outer.east",
            Self::BoxShellRim => "shell.box.rim",
            Self::PlanarOffsetFace => "planar_offset.face",
            Self::SweepStart => "sweep.start",
            Self::SweepEnd => "sweep.end",
            Self::SweepSide0 => "sweep.side.0",
            Self::SweepSide1 => "sweep.side.1",
            Self::SweepSide2 => "sweep.side.2",
            Self::SweepSide3 => "sweep.side.3",
            Self::LoftStart => "loft.start",
            Self::LoftEnd => "loft.end",
            Self::LoftSide => "loft.side",
        }
    }

    #[must_use]
    pub const fn source_element_id(self) -> &'static str {
        match self {
            Self::Top | Self::Bottom => "profile.face",
            Self::East => "profile.edge.east",
            Self::West => "profile.edge.west",
            Self::CircleSide => "profile.edge.circle",
            Self::ArcSide => "profile.edge.arc.0",
            Self::LinearSide => "profile.edge.line.0",
            Self::CutCircle => "cut_profile.edge.circle",
            Self::CutLinear => "cut_profile.edge.line.0",
            Self::CutArc => "cut_profile.edge.arc.0",
            Self::CutWest => "cut_profile.edge.west",
            Self::CutEast => "cut_profile.edge.east",
            Self::CutSouth => "cut_profile.edge.south",
            Self::CutNorth => "cut_profile.edge.north",
            Self::PocketFloor => "pocket_profile.face",
            Self::PocketWest => "pocket_profile.edge.west",
            Self::PocketEast => "pocket_profile.edge.east",
            Self::PocketSouth => "pocket_profile.edge.south",
            Self::PocketNorth => "pocket_profile.edge.north",
            Self::RevolveBottom => "profile.edge.0",
            Self::RevolveBody => "profile.edge.1",
            Self::RevolveShoulder => "profile.edge.2",
            Self::RevolveNeck => "profile.edge.3",
            Self::RevolveMouth => "profile.edge.4",
            Self::RevolveSide0 => "profile.edge.0",
            Self::RevolveSide1 => "profile.edge.1",
            Self::RevolveStart | Self::RevolveEnd => "profile.face",
            Self::ShellOuterBottom => "revolve.face.bottom",
            Self::ShellOuterBody => "revolve.face.body",
            Self::ShellOuterShoulder => "revolve.face.shoulder",
            Self::ShellOuterNeck => "revolve.face.neck",
            Self::ShellRim => "revolve.face.mouth",
            Self::ShellInnerBottom => "shell.offset.bottom",
            Self::ShellInnerBody => "shell.offset.body",
            Self::ShellInnerShoulder => "shell.offset.shoulder",
            Self::ShellInnerNeck => "shell.offset.neck",
            Self::BoxShellOuterBottom => "extrusion.bottom",
            Self::BoxShellOuterEast => "extrusion.side(profile_edge=east)",
            Self::BoxShellRim => "extrusion.top",
            Self::PlanarOffsetFace | Self::SweepStart | Self::SweepEnd => "profile.face",
            Self::SweepSide0 => "profile.edge.0",
            Self::SweepSide1 => "profile.edge.1",
            Self::SweepSide2 => "profile.edge.2",
            Self::SweepSide3 => "profile.edge.3",
            Self::LoftStart | Self::LoftEnd => "profile.face",
            Self::LoftSide => "profile.edge.spline",
        }
    }

    #[must_use]
    pub const fn expected_type(self) -> &'static str {
        match self {
            Self::Top
            | Self::Bottom
            | Self::East
            | Self::West
            | Self::LinearSide
            | Self::CutLinear
            | Self::CutWest
            | Self::CutEast
            | Self::CutSouth
            | Self::CutNorth
            | Self::PocketFloor
            | Self::PocketWest
            | Self::PocketEast
            | Self::PocketSouth
            | Self::PocketNorth
            | Self::PlanarOffsetFace
            | Self::SweepStart
            | Self::SweepEnd
            | Self::SweepSide0
            | Self::SweepSide1
            | Self::SweepSide2
            | Self::SweepSide3
            | Self::LoftStart
            | Self::LoftEnd => "planar_face",
            Self::CircleSide | Self::CutCircle => "cylindrical_face",
            Self::ArcSide
            | Self::CutArc
            | Self::RevolveBottom
            | Self::RevolveBody
            | Self::RevolveShoulder
            | Self::RevolveNeck
            | Self::RevolveMouth
            | Self::RevolveSide0
            | Self::RevolveSide1
            | Self::RevolveStart
            | Self::RevolveEnd
            | Self::ShellOuterBottom
            | Self::ShellOuterBody
            | Self::ShellOuterShoulder
            | Self::ShellOuterNeck
            | Self::ShellRim
            | Self::ShellInnerBottom
            | Self::ShellInnerBody
            | Self::ShellInnerShoulder
            | Self::ShellInnerNeck => "face",
            Self::BoxShellOuterBottom | Self::BoxShellOuterEast | Self::BoxShellRim => {
                "planar_face"
            }
            Self::LoftSide => "face",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExactEdgeRole {
    NorthEastVertical,
}

impl ExactEdgeRole {
    #[must_use]
    pub const fn semantic_role(self) -> &'static str {
        match self {
            Self::NorthEastVertical => "extrusion.edge(profile_vertex=north_east)",
        }
    }

    #[must_use]
    pub const fn source_element_id(self) -> &'static str {
        match self {
            Self::NorthEastVertical => "profile.vertex.north_east",
        }
    }

    #[must_use]
    pub const fn expected_type(self) -> &'static str {
        "edge"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceStability {
    Guaranteed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodySubshapeRef {
    pub schema: String,
    pub document_id: DocumentId,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub producer_feature_id: FeatureId,
    pub semantic_role: String,
    pub source_element_id: String,
    pub expected_type: String,
    pub expected_cardinality: u32,
    pub stability: ReferenceStability,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
    pub lineage_digest: String,
    pub corroborating_geometry_fingerprint: String,
}

impl BodySubshapeRef {
    #[must_use]
    pub fn role(&self) -> Option<ExactFaceRole> {
        [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
            ExactFaceRole::West,
            ExactFaceRole::CircleSide,
            ExactFaceRole::ArcSide,
            ExactFaceRole::LinearSide,
            ExactFaceRole::CutCircle,
            ExactFaceRole::CutLinear,
            ExactFaceRole::CutArc,
            ExactFaceRole::CutWest,
            ExactFaceRole::CutEast,
            ExactFaceRole::CutSouth,
            ExactFaceRole::CutNorth,
            ExactFaceRole::PocketFloor,
            ExactFaceRole::PocketWest,
            ExactFaceRole::PocketEast,
            ExactFaceRole::PocketSouth,
            ExactFaceRole::PocketNorth,
            ExactFaceRole::RevolveBottom,
            ExactFaceRole::RevolveBody,
            ExactFaceRole::RevolveShoulder,
            ExactFaceRole::RevolveNeck,
            ExactFaceRole::RevolveMouth,
            ExactFaceRole::RevolveSide0,
            ExactFaceRole::RevolveSide1,
            ExactFaceRole::RevolveStart,
            ExactFaceRole::RevolveEnd,
            ExactFaceRole::ShellOuterBottom,
            ExactFaceRole::ShellOuterBody,
            ExactFaceRole::ShellOuterShoulder,
            ExactFaceRole::ShellOuterNeck,
            ExactFaceRole::ShellRim,
            ExactFaceRole::ShellInnerBottom,
            ExactFaceRole::ShellInnerBody,
            ExactFaceRole::ShellInnerShoulder,
            ExactFaceRole::ShellInnerNeck,
            ExactFaceRole::BoxShellOuterBottom,
            ExactFaceRole::BoxShellOuterEast,
            ExactFaceRole::BoxShellRim,
            ExactFaceRole::PlanarOffsetFace,
            ExactFaceRole::SweepStart,
            ExactFaceRole::SweepEnd,
            ExactFaceRole::SweepSide0,
            ExactFaceRole::SweepSide1,
            ExactFaceRole::SweepSide2,
            ExactFaceRole::SweepSide3,
            ExactFaceRole::LoftStart,
            ExactFaceRole::LoftEnd,
            ExactFaceRole::LoftSide,
        ]
        .into_iter()
        .find(|role| {
            self.semantic_role == role.semantic_role()
                && self.source_element_id == role.source_element_id()
        })
    }

    #[must_use]
    pub fn edge_role(&self) -> Option<ExactEdgeRole> {
        [ExactEdgeRole::NorthEastVertical].into_iter().find(|role| {
            self.semantic_role == role.semantic_role()
                && self.source_element_id == role.source_element_id()
        })
    }

    #[must_use]
    pub fn has_valid_lineage(&self) -> bool {
        self.schema == BODY_SUBSHAPE_REF_SCHEMA_V1
            && self.expected_cardinality == 1
            && (self
                .role()
                .is_some_and(|role| self.expected_type == role.expected_type())
                || self
                    .edge_role()
                    .is_some_and(|role| self.expected_type == role.expected_type()))
            && self.lineage_digest == reference_lineage_digest(self)
    }

    #[must_use]
    pub fn matches_planar_offset_request(&self, request: &ExactPlanarOffsetRequest) -> bool {
        self.has_valid_lineage()
            && self.role() == Some(ExactFaceRole::PlanarOffsetFace)
            && self.document_id == request.document_id
            && self.definition_id == request.definition_id
            && self.profile_feature_id == request.profile_feature_id
            && self.producer_feature_id == request.offset_feature_id
            && self.canonical_input_digest == request.canonical_input_digest
            && self.evaluator == request.evaluator()
            && !self.exact_input_digest.is_empty()
            && !self.result_fingerprint.is_empty()
            && !self.backend.is_empty()
            && !self.tolerance.is_empty()
            && !self.corroborating_geometry_fingerprint.is_empty()
    }

    #[must_use]
    pub fn matches_sweep_request(&self, request: &ExactSweepRequest) -> bool {
        self.has_valid_lineage()
            && matches!(
                self.role(),
                Some(
                    ExactFaceRole::SweepStart
                        | ExactFaceRole::SweepEnd
                        | ExactFaceRole::SweepSide0
                        | ExactFaceRole::SweepSide1
                        | ExactFaceRole::SweepSide2
                        | ExactFaceRole::SweepSide3
                )
            )
            && self.document_id == request.document_id
            && self.definition_id == request.definition_id
            && self.profile_feature_id == request.profile_feature_id
            && self.producer_feature_id == request.sweep_feature_id
            && self.canonical_input_digest == request.canonical_input_digest
            && self.evaluator == request.evaluator()
            && !self.exact_input_digest.is_empty()
            && !self.result_fingerprint.is_empty()
            && !self.backend.is_empty()
            && !self.tolerance.is_empty()
            && !self.corroborating_geometry_fingerprint.is_empty()
    }

    #[must_use]
    pub fn matches_loft_request(&self, request: &ExactLoftRequest) -> bool {
        let expected_profile = match self.role() {
            Some(ExactFaceRole::LoftStart | ExactFaceRole::LoftSide) => request
                .sections
                .first()
                .map(|section| section.profile_feature_id),
            Some(ExactFaceRole::LoftEnd) => request
                .sections
                .last()
                .map(|section| section.profile_feature_id),
            _ => None,
        };
        self.has_valid_lineage()
            && expected_profile == Some(self.profile_feature_id)
            && self.document_id == request.document_id
            && self.definition_id == request.definition_id
            && self.producer_feature_id == request.loft_feature_id
            && self.canonical_input_digest == request.canonical_input_digest
            && self.evaluator == request.evaluator()
            && !self.exact_input_digest.is_empty()
            && !self.result_fingerprint.is_empty()
            && !self.backend.is_empty()
            && !self.tolerance.is_empty()
            && !self.corroborating_geometry_fingerprint.is_empty()
    }

    #[must_use]
    pub fn matches_durable_request_identity(&self, request: &ExactFeatureChainRequest) -> bool {
        let profile_matches = if let Some(role) = self.role() {
            request.expected_face_roles().contains(&role)
                && request.profile_feature_id_for_role(role) == Some(self.profile_feature_id)
        } else {
            self.edge_role() == Some(ExactEdgeRole::NorthEastVertical)
                && request.profile_feature_id == self.profile_feature_id
        };
        self.has_valid_lineage()
            && profile_matches
            && self.document_id == request.document_id
            && self.definition_id == request.definition_id
            && self.producer_feature_id == request.producer_feature_id()
    }

    #[must_use]
    pub fn matches_request(&self, request: &ExactFeatureChainRequest) -> bool {
        self.matches_request_digest(request, &request.canonical_input_digest)
    }

    #[must_use]
    pub fn matches_legacy_request(&self, request: &ExactFeatureChainRequest) -> bool {
        self.matches_request_digest(request, &request.legacy_canonical_input_digest)
    }

    fn matches_request_digest(
        &self,
        request: &ExactFeatureChainRequest,
        canonical_input_digest: &str,
    ) -> bool {
        self.matches_durable_request_identity(request)
            && self.canonical_input_digest == canonical_input_digest
            && self.evaluator == request.evaluator()
            && !self.exact_input_digest.is_empty()
            && !self.result_fingerprint.is_empty()
            && !self.backend.is_empty()
            && !self.tolerance.is_empty()
            && !self.corroborating_geometry_fingerprint.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactReferenceQuarantineReason {
    InvalidLineage,
    WrongDocument,
    IncompatibleEvaluationEnvelope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactReferenceResolution {
    Resolved {
        reference: Box<BodySubshapeRef>,
    },
    Ambiguous {
        candidate_count: usize,
    },
    Lost,
    Quarantined {
        reason: ExactReferenceQuarantineReason,
    },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BodyResultIdentity {
    pub schema: String,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub extrusion_feature_id: FeatureId,
    pub producer_feature_id: FeatureId,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactVertex {
    pub position_mm: [f64; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactTriangle {
    pub vertex_indices: [u32; 3],
    pub face_role: Option<ExactFaceRole>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactRenderPackage {
    pub identity: BodyResultIdentity,
    pub bounds_mm: [[f64; 3]; 2],
    pub vertices: Vec<ExactVertex>,
    pub triangles: Vec<ExactTriangle>,
    pub references: Vec<BodySubshapeRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedExactPackage {
    pub identity: BodyResultIdentity,
    pub source_sha256: [u8; 32],
    pub source_bytes: Vec<u8>,
    pub solid_count: u32,
    pub topology_counts: Option<[u32; 5]>,
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub vertices: Vec<ExactVertex>,
    pub triangles: Vec<ExactTriangle>,
    pub triangle_face_ordinals: Vec<u32>,
    pub topological_references: Vec<TopologicalElementRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactBRepGraphPackage {
    pub identity: BodyResultIdentity,
    pub graph: ExactBRepGraph,
    pub volume_mm3: f64,
    pub topology_counts: [u32; 5],
    pub bounds_mm: [[f64; 3]; 2],
    pub vertices: Vec<ExactVertex>,
    pub triangles: Vec<ExactTriangle>,
    pub triangle_face_ordinals: Vec<u32>,
    pub topological_references: Vec<TopologicalElementRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactBRepGraphWorkerEvidence {
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub volume_mm3: f64,
    pub topology_counts: [u32; 5],
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
}

impl ExactBRepGraphPackage {
    pub fn from_worker_evidence(
        graph: &ExactBRepGraph,
        evidence: ExactBRepGraphWorkerEvidence,
        mesh: &StepImportMesh,
    ) -> Result<Self, ExactProductError> {
        graph
            .validate()
            .map_err(|_| ExactProductError::InvalidWorkerEvidence)?;
        let vertex_count = mesh.vertices_mm.len();
        if evidence.exact_input_digest.is_empty()
            || evidence.result_fingerprint.is_empty()
            || evidence.backend.is_empty()
            || evidence.tolerance.is_empty()
            || !evidence.volume_mm3.is_finite()
            || evidence.volume_mm3 <= 0.0
            || evidence.topology_counts.contains(&0)
            || evidence
                .bounds_mm
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
            || (0..3).any(|axis| evidence.bounds_mm[0][axis] >= evidence.bounds_mm[1][axis])
            || mesh.triangles.is_empty()
            || !mesh.is_within_bounds(evidence.bounds_mm, IMPORTED_MESH_BOUNDS_TOLERANCE_MM)
            || mesh.triangles.iter().any(|triangle| {
                triangle.face_ordinal >= evidence.topology_counts[2]
                    || triangle
                        .vertex_indices
                        .iter()
                        .any(|index| *index as usize >= vertex_count)
            })
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let producer_feature_id = FeatureId(graph.producer_feature_id);
        let profile_feature_id = graph
            .profiles
            .first()
            .map_or(producer_feature_id, |profile| {
                FeatureId(profile.source_feature_id)
            });
        let identity = BodyResultIdentity {
            schema: EXACT_PRODUCT_SCHEMA_V1.to_owned(),
            document_id: DocumentId(graph.document_id),
            source_revision: graph.source_revision,
            source_digest: graph.source_digest.clone(),
            definition_id: DefinitionId(graph.definition_id),
            profile_feature_id,
            extrusion_feature_id: producer_feature_id,
            producer_feature_id,
            canonical_input_digest: graph.canonical_input_digest.clone(),
            exact_input_digest: evidence.exact_input_digest,
            result_fingerprint: evidence.result_fingerprint,
            evaluator: EXACT_BREP_GRAPH_EVALUATOR_V1.to_owned(),
            backend: evidence.backend,
            tolerance: evidence.tolerance,
        };
        let topological_references =
            publish_generated_topological_references(&identity, evidence.topology_counts)
                .map_err(|_| ExactProductError::InvalidWorkerEvidence)?;
        Ok(Self {
            identity,
            graph: graph.clone(),
            volume_mm3: evidence.volume_mm3,
            topology_counts: evidence.topology_counts,
            bounds_mm: evidence.bounds_mm,
            vertices: mesh
                .vertices_mm
                .iter()
                .copied()
                .map(|position_mm| ExactVertex { position_mm })
                .collect(),
            triangles: mesh
                .triangles
                .iter()
                .map(|triangle| ExactTriangle {
                    vertex_indices: triangle.vertex_indices,
                    face_role: None,
                })
                .collect(),
            triangle_face_ordinals: mesh
                .triangles
                .iter()
                .map(|triangle| triangle.face_ordinal)
                .collect(),
            topological_references,
        })
    }

    fn matches_graph(&self, graph: &ExactBRepGraph) -> bool {
        self.identity.document_id == DocumentId(graph.document_id)
            && self.identity.definition_id == DefinitionId(graph.definition_id)
            && self.identity.producer_feature_id == FeatureId(graph.producer_feature_id)
            && self.identity.canonical_input_digest == graph.canonical_input_digest
            && self.identity.evaluator == EXACT_BREP_GRAPH_EVALUATOR_V1
            && self.graph == *graph
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && ExactBRepGraph::from_snapshot(
                snapshot,
                self.identity.definition_id,
                self.identity.producer_feature_id,
            )
            .is_ok_and(|graph| self.matches_graph(&graph))
    }

    #[must_use]
    pub fn rebound_to(&self, snapshot: &Snapshot) -> Option<Self> {
        let graph = ExactBRepGraph::from_snapshot(
            snapshot,
            self.identity.definition_id,
            self.identity.producer_feature_id,
        )
        .ok()?;
        if graph.graph_digest != self.graph.graph_digest {
            return None;
        }
        let mut rebound = self.clone();
        rebound.identity.source_revision = snapshot.revision_id();
        rebound.identity.source_digest = snapshot.canonical_digest();
        rebound.identity.canonical_input_digest = graph.canonical_input_digest.clone();
        rebound.graph = graph;
        Some(rebound)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExactBodyPackage {
    Rectangle(ExactRenderPackage),
    Revolve(crate::exact_revolve::ExactRevolvePackage),
    Graph(ExactBRepGraphPackage),
    Imported(ImportedExactPackage),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactMeshExport {
    pub mesh_obj: String,
    pub loss_report: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactStlExport {
    pub mesh_stl: String,
    pub loss_report: String,
}

pub trait ExactBodyView {
    fn bounds_mm(&self) -> [[f64; 3]; 2];
    fn vertex_count(&self) -> usize;
    fn vertex_position_mm(&self, index: usize) -> [f64; 3];
    fn triangle_count(&self) -> usize;
    fn triangle_indices(&self, index: usize) -> [u32; 3];
    fn triangle_group(&self, index: usize) -> &'static str;
    fn triangle_group_name(&self, index: usize) -> String {
        self.triangle_group(index).to_owned()
    }
    fn tolerance(&self) -> &str;
    fn source_digest(&self) -> &str;
    fn producer_identity(&self) -> String;
    fn result_fingerprint(&self) -> &str;

    #[must_use]
    fn mesh_export(&self, transform: Transform) -> ExactMeshExport {
        mesh_export_from_view(self, transform)
    }
}

/// How far a tessellated vertex may sit outside the committed exact bounds.
///
/// A tessellation interpolates curved faces, so it stays inside the exact
/// bounds; this slack only absorbs floating-point noise at the extremes.
const IMPORTED_MESH_BOUNDS_TOLERANCE_MM: f64 = 1.0e-6;

impl ImportedExactPackage {
    /// Build the render product of an imported exact body from its canonical
    /// specification, its content-addressed source bytes, and the derived
    /// display mesh the isolated worker tessellated for it.
    ///
    /// The mesh is display-only: it never becomes canonical state, and it is
    /// refused unless every vertex lies inside the bounds the import receipt
    /// already committed to, so a swapped or stale mesh cannot be shown.
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        source_bytes: Vec<u8>,
        mesh: &crate::import::StepImportMesh,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        let [feature_id] = definition.feature_ids() else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        let feature = snapshot
            .feature(*feature_id)
            .ok_or(ExactProductError::UnsupportedDefinition)?;
        let FeatureKind::ImportedExactBody(spec) = feature.kind() else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        let source_sha256: [u8; 32] = Sha256::digest(&source_bytes).into();
        if source_bytes.len() as u64 != spec.source_byte_len || source_sha256 != spec.source_sha256
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        if !mesh.is_within_bounds(spec.bounds_mm, IMPORTED_MESH_BOUNDS_TOLERANCE_MM)
            || spec.topology_counts.is_some_and(|counts| {
                mesh.triangles
                    .iter()
                    .any(|triangle| triangle.face_ordinal >= counts[2])
            })
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let vertices = mesh
            .vertices_mm
            .iter()
            .map(|position| ExactVertex {
                position_mm: *position,
            })
            .collect::<Vec<_>>();
        let triangles = mesh
            .triangles
            .iter()
            .map(|triangle| ExactTriangle {
                vertex_indices: triangle.vertex_indices,
                face_role: None,
            })
            .collect::<Vec<_>>();
        if triangles.is_empty() {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let source_digest = snapshot.canonical_digest();
        let source_identity = spec
            .source_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let identity = BodyResultIdentity {
            schema: EXACT_PRODUCT_SCHEMA_V1.to_owned(),
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id: *feature_id,
            extrusion_feature_id: *feature_id,
            producer_feature_id: *feature_id,
            canonical_input_digest: source_identity.clone(),
            exact_input_digest: source_identity,
            result_fingerprint: spec.result_fingerprint.clone(),
            evaluator: "ketchup.imported-step-evaluator.v1".to_owned(),
            backend: spec.backend.clone(),
            tolerance: spec.tolerance.clone(),
        };
        let topological_references = spec
            .topology_counts
            .map(|counts| {
                publish_imported_topological_references(&identity, &source_sha256, counts)
            })
            .transpose()
            .map_err(|_| ExactProductError::InvalidWorkerEvidence)?
            .unwrap_or_default();
        Ok(Self {
            identity,
            source_sha256,
            source_bytes,
            solid_count: spec.solid_count,
            topology_counts: spec.topology_counts,
            volume_mm3: spec.volume_mm3,
            bounds_mm: spec.bounds_mm,
            vertices,
            triangles,
            triangle_face_ordinals: mesh
                .triangles
                .iter()
                .map(|triangle| triangle.face_ordinal)
                .collect(),
            topological_references,
        })
    }

    /// Whether `snapshot` still carries exactly the canonical evidence this
    /// product was built from, ignoring which revision published it.
    ///
    /// An imported exact body is content-addressed by its source bytes and by
    /// the worker fingerprint that produced it, so nothing it depends on lives
    /// outside its own feature.
    fn matches_snapshot_evidence(&self, snapshot: &Snapshot) -> bool {
        self.identity.document_id == snapshot.document_id()
            && snapshot
                .feature(self.identity.producer_feature_id)
                .is_some_and(|feature| match feature.kind() {
                    FeatureKind::ImportedExactBody(spec) => {
                        feature.definition_id() == self.identity.definition_id
                            && spec.source_sha256 == self.source_sha256
                            && spec.source_byte_len == self.source_bytes.len() as u64
                            && spec.result_fingerprint == self.identity.result_fingerprint
                            && spec.solid_count == self.solid_count
                            && spec.topology_counts == self.topology_counts
                            && spec.volume_mm3 == self.volume_mm3
                            && spec.bounds_mm == self.bounds_mm
                            && spec.backend == self.identity.backend
                            && spec.tolerance == self.identity.tolerance
                    }
                    _ => false,
                })
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && self.matches_snapshot_evidence(snapshot)
    }

    /// The same product rebound to `snapshot`, or `None` when `snapshot` no
    /// longer carries the evidence it was built from.
    ///
    /// Editing anything else in the document — moving the occurrence, for
    /// instance — publishes a new revision without touching this import, and
    /// re-deriving it costs an isolated worker round trip.
    #[must_use]
    pub fn rebound_to(&self, snapshot: &Snapshot) -> Option<Self> {
        if !self.matches_snapshot_evidence(snapshot) {
            return None;
        }
        let mut rebound = self.clone();
        rebound.identity.source_revision = snapshot.revision_id();
        rebound.identity.source_digest = snapshot.canonical_digest();
        Some(rebound)
    }
}

impl ExactBodyPackage {
    #[must_use]
    pub fn definition_id(&self) -> DefinitionId {
        match self {
            Self::Rectangle(package) => package.identity.definition_id,
            Self::Revolve(package) => package.identity.definition_id,
            Self::Graph(package) => package.identity.definition_id,
            Self::Imported(package) => package.identity.definition_id,
        }
    }

    #[must_use]
    pub fn producer_feature_id(&self) -> FeatureId {
        match self {
            Self::Rectangle(package) => package.identity.producer_feature_id,
            Self::Revolve(package) => package.identity.producer_feature_id,
            Self::Graph(package) => package.identity.producer_feature_id,
            Self::Imported(package) => package.identity.producer_feature_id,
        }
    }

    #[must_use]
    pub fn result_key(&self) -> ExactResultKey {
        match self {
            Self::Rectangle(package) => ExactResultKey {
                document_id: package.identity.document_id,
                source_revision: package.identity.source_revision,
                source_digest: package.identity.source_digest.clone(),
                definition_id: package.identity.definition_id,
                producer_feature_id: package.identity.producer_feature_id,
                canonical_input_digest: package.identity.canonical_input_digest.clone(),
                exact_input_digest: package.identity.exact_input_digest.clone(),
                evaluator: package.identity.evaluator.clone(),
                backend: package.identity.backend.clone(),
                tolerance: package.identity.tolerance.clone(),
                schema: package.identity.schema.clone(),
                result_fingerprint: package.identity.result_fingerprint.clone(),
            },
            Self::Revolve(package) => ExactResultKey {
                document_id: package.identity.document_id,
                source_revision: package.identity.source_revision,
                source_digest: package.identity.source_digest.clone(),
                definition_id: package.identity.definition_id,
                producer_feature_id: package.identity.producer_feature_id,
                canonical_input_digest: package.identity.canonical_input_digest.clone(),
                exact_input_digest: package.identity.exact_input_digest.clone(),
                evaluator: package.identity.evaluator.clone(),
                backend: package.identity.backend.clone(),
                tolerance: package.identity.tolerance.clone(),
                schema: package.identity.schema.clone(),
                result_fingerprint: package.identity.result_fingerprint.clone(),
            },
            Self::Graph(package) => ExactResultKey {
                document_id: package.identity.document_id,
                source_revision: package.identity.source_revision,
                source_digest: package.identity.source_digest.clone(),
                definition_id: package.identity.definition_id,
                producer_feature_id: package.identity.producer_feature_id,
                canonical_input_digest: package.identity.canonical_input_digest.clone(),
                exact_input_digest: package.identity.exact_input_digest.clone(),
                evaluator: package.identity.evaluator.clone(),
                backend: package.identity.backend.clone(),
                tolerance: package.identity.tolerance.clone(),
                schema: package.identity.schema.clone(),
                result_fingerprint: package.identity.result_fingerprint.clone(),
            },
            Self::Imported(package) => ExactResultKey {
                document_id: package.identity.document_id,
                source_revision: package.identity.source_revision,
                source_digest: package.identity.source_digest.clone(),
                definition_id: package.identity.definition_id,
                producer_feature_id: package.identity.producer_feature_id,
                canonical_input_digest: package.identity.canonical_input_digest.clone(),
                exact_input_digest: package.identity.exact_input_digest.clone(),
                evaluator: package.identity.evaluator.clone(),
                backend: package.identity.backend.clone(),
                tolerance: package.identity.tolerance.clone(),
                schema: package.identity.schema.clone(),
                result_fingerprint: package.identity.result_fingerprint.clone(),
            },
        }
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        match self {
            Self::Rectangle(package) => package.is_current(snapshot),
            Self::Revolve(package) => package.is_current(snapshot),
            Self::Graph(package) => package.is_current(snapshot),
            Self::Imported(package) => package.is_current(snapshot),
        }
    }

    #[must_use]
    pub fn rebound_to(&self, snapshot: &Snapshot) -> Option<Self> {
        match self {
            Self::Rectangle(package) => {
                let request = ExactFeatureChainRequest::from_snapshot_for_producer(
                    snapshot,
                    package.identity.definition_id,
                    package.identity.producer_feature_id,
                )
                .ok()?;
                if request.canonical_input_digest != package.identity.canonical_input_digest
                    || request.evaluator() != package.identity.evaluator
                {
                    return None;
                }
                let mut rebound = package.clone();
                rebound.identity.source_revision = snapshot.revision_id();
                rebound.identity.source_digest = snapshot.canonical_digest();
                rebound.validate_for_request(&request).ok()?;
                Some(Self::Rectangle(rebound))
            }
            Self::Revolve(package) => {
                let request = crate::exact_revolve::ExactRevolveRequest::from_snapshot(
                    snapshot,
                    package.identity.definition_id,
                )
                .ok()?;
                if request.producer_feature_id() != package.identity.producer_feature_id
                    || request.canonical_input_digest_for_envelope(
                        package.identity.source_revision,
                        &package.identity.source_digest,
                    ) != package.identity.canonical_input_digest
                    || request.evaluator() != package.identity.evaluator
                {
                    return None;
                }
                let mut rebound = package.clone();
                rebound.identity.source_revision = snapshot.revision_id();
                rebound.identity.source_digest = snapshot.canonical_digest();
                rebound.identity.canonical_input_digest = request.canonical_input_digest.clone();
                for reference in &mut rebound.references {
                    reference.canonical_input_digest = request.canonical_input_digest.clone();
                }
                rebound.validate_for_request(&request).ok()?;
                Some(Self::Revolve(rebound))
            }
            Self::Graph(package) => package.rebound_to(snapshot).map(Self::Graph),
            Self::Imported(package) => package.rebound_to(snapshot).map(Self::Imported),
        }
    }

    #[must_use]
    pub fn bounds_mm(&self) -> [[f64; 3]; 2] {
        match self {
            Self::Rectangle(package) => package.bounds_mm,
            Self::Revolve(package) => package.bounds_mm,
            Self::Graph(package) => package.bounds_mm,
            Self::Imported(package) => package.bounds_mm,
        }
    }

    #[must_use]
    pub fn vertices(&self) -> &[ExactVertex] {
        match self {
            Self::Rectangle(package) => &package.vertices,
            Self::Revolve(package) => &package.vertices,
            Self::Graph(package) => &package.vertices,
            Self::Imported(package) => &package.vertices,
        }
    }

    #[must_use]
    pub fn triangles(&self) -> &[ExactTriangle] {
        match self {
            Self::Rectangle(package) => &package.triangles,
            Self::Revolve(package) => &package.triangles,
            Self::Graph(package) => &package.triangles,
            Self::Imported(package) => &package.triangles,
        }
    }

    #[must_use]
    pub fn references(&self) -> &[BodySubshapeRef] {
        match self {
            Self::Rectangle(package) => &package.references,
            Self::Revolve(package) => &package.references,
            Self::Graph(_) | Self::Imported(_) => &[],
        }
    }

    #[must_use]
    pub fn topological_references(&self) -> &[TopologicalElementRef] {
        match self {
            Self::Graph(package) => &package.topological_references,
            Self::Imported(package) => &package.topological_references,
            Self::Rectangle(_) | Self::Revolve(_) => &[],
        }
    }

    #[must_use]
    pub fn topological_reference(
        &self,
        kind: TopologicalElementKind,
        ordinal: u32,
    ) -> Option<&TopologicalElementRef> {
        self.topological_references()
            .iter()
            .filter(|reference| reference.kind == kind)
            .nth(ordinal as usize)
    }

    #[must_use]
    pub fn topological_reference_for_triangle(
        &self,
        triangle_index: usize,
    ) -> Option<&TopologicalElementRef> {
        let face_ordinal = match self {
            Self::Graph(package) => package.triangle_face_ordinals.get(triangle_index),
            Self::Imported(package) => package.triangle_face_ordinals.get(triangle_index),
            Self::Rectangle(_) | Self::Revolve(_) => None,
        }?;
        self.topological_reference(TopologicalElementKind::Face, *face_ordinal)
    }

    #[must_use]
    pub fn reference(&self, role: ExactFaceRole) -> Option<&BodySubshapeRef> {
        self.references()
            .iter()
            .find(|reference| reference.role() == Some(role))
    }

    #[must_use]
    pub fn mesh_export(&self, transform: Transform) -> ExactMeshExport {
        mesh_export_from_view(self, transform)
    }

    #[must_use]
    pub fn revolve(&self) -> Option<&crate::exact_revolve::ExactRevolvePackage> {
        match self {
            Self::Revolve(package) => Some(package),
            Self::Rectangle(_) | Self::Graph(_) | Self::Imported(_) => None,
        }
    }

    pub fn detached_mesh_conversion_batch(
        &self,
        snapshot: &Snapshot,
        destination_definition_id: DefinitionId,
        destination_definition_name: impl Into<String>,
        destination_feature_id: FeatureId,
        destination_feature_name: impl Into<String>,
    ) -> Result<CommandBatch, ExactProductError> {
        if !self.is_current(snapshot) {
            return Err(ExactProductError::StaleResult);
        }
        if matches!(self, Self::Imported(_)) {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let key = self.result_key();
        let spec = MeshBodySpec {
            schema: MESH_BODY_SCHEMA_V1.to_owned(),
            vertices_mm: self
                .vertices()
                .iter()
                .map(|vertex| vertex.position_mm)
                .collect(),
            triangles: self
                .triangles()
                .iter()
                .map(|triangle| triangle.vertex_indices)
                .collect(),
            authority: MeshAuthority::ExactConversion(ExactToMeshConversion {
                source_document_id: key.document_id,
                source_revision: key.source_revision,
                source_digest: key.source_digest,
                source_definition_id: key.definition_id,
                source_feature_id: key.producer_feature_id,
                source_result_fingerprint: key.result_fingerprint,
                source_evaluator: key.evaluator,
                source_backend: key.backend,
                source_tolerance: key.tolerance.clone(),
                tessellation_tolerance: key.tolerance,
                destination_definition_id,
                destination_feature_id,
                unsupported_semantics: vec![
                    "analytic_surfaces".to_owned(),
                    "canonical_exact_features_rules_dimensions".to_owned(),
                    "durable_exact_subshape_references".to_owned(),
                    "exact_topology".to_owned(),
                ],
                exact_reference_consequence: ExactReferenceConversionConsequence::Lost,
            }),
        };
        Ok(CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: destination_definition_id,
                name: destination_definition_name.into(),
            },
            CanonicalCommand::CreateFeature {
                id: destination_feature_id,
                definition_id: destination_definition_id,
                name: destination_feature_name.into(),
                kind: FeatureKind::MeshBody(spec),
            },
        ]))
    }
}

impl ExactBodyView for ExactBodyPackage {
    fn bounds_mm(&self) -> [[f64; 3]; 2] {
        self.bounds_mm()
    }

    fn vertex_count(&self) -> usize {
        self.vertices().len()
    }

    fn vertex_position_mm(&self, index: usize) -> [f64; 3] {
        self.vertices()[index].position_mm
    }

    fn triangle_count(&self) -> usize {
        self.triangles().len()
    }

    fn triangle_indices(&self, index: usize) -> [u32; 3] {
        self.triangles()[index].vertex_indices
    }

    fn triangle_group(&self, index: usize) -> &'static str {
        self.triangles()[index]
            .face_role
            .map_or("unreferenced", ExactFaceRole::semantic_role)
    }

    fn triangle_group_name(&self, index: usize) -> String {
        match self {
            Self::Graph(package) => package.triangle_face_ordinals.get(index).map_or_else(
                || "unreferenced".to_owned(),
                |ordinal| format!("topological.face.{ordinal}"),
            ),
            Self::Imported(package) => package.triangle_face_ordinals.get(index).map_or_else(
                || "unreferenced".to_owned(),
                |ordinal| format!("imported.face.{ordinal}"),
            ),
            Self::Rectangle(_) | Self::Revolve(_) => self.triangle_group(index).to_owned(),
        }
    }

    fn tolerance(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.tolerance,
            Self::Revolve(package) => &package.identity.tolerance,
            Self::Graph(package) => &package.identity.tolerance,
            Self::Imported(package) => &package.identity.tolerance,
        }
    }

    fn source_digest(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.source_digest,
            Self::Revolve(package) => &package.identity.source_digest,
            Self::Graph(package) => &package.identity.source_digest,
            Self::Imported(package) => &package.identity.source_digest,
        }
    }

    fn producer_identity(&self) -> String {
        format!("producer_feature_id={}", self.producer_feature_id().0)
    }

    fn result_fingerprint(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.result_fingerprint,
            Self::Revolve(package) => &package.identity.result_fingerprint,
            Self::Graph(package) => &package.identity.result_fingerprint,
            Self::Imported(package) => &package.identity.result_fingerprint,
        }
    }
}

#[cfg(feature = "named-product-fixtures")]
impl ExactBodyView for BeamExactPiecePackage {
    fn bounds_mm(&self) -> [[f64; 3]; 2] {
        [self.bounds_mm.min(), self.bounds_mm.max()]
    }

    fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    fn vertex_position_mm(&self, index: usize) -> [f64; 3] {
        self.vertices[index].position_mm
    }

    fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    fn triangle_indices(&self, index: usize) -> [u32; 3] {
        self.triangles[index].vertex_indices
    }

    fn triangle_group(&self, _index: usize) -> &'static str {
        "unreferenced"
    }

    fn tolerance(&self) -> &str {
        &self.identity.tolerance
    }

    fn source_digest(&self) -> &str {
        &self.identity.source_digest
    }

    fn producer_identity(&self) -> String {
        format!("producer_piece_key={}", self.identity.piece_key)
    }

    fn result_fingerprint(&self) -> &str {
        &self.identity.result_fingerprint
    }
}

fn mesh_export_from_view(
    view: &(impl ExactBodyView + ?Sized),
    transform: Transform,
) -> ExactMeshExport {
    let matrix = transform.matrix();
    let determinant = matrix[0] * (matrix[5] * matrix[10] - matrix[6] * matrix[9])
        - matrix[1] * (matrix[4] * matrix[10] - matrix[6] * matrix[8])
        + matrix[2] * (matrix[4] * matrix[9] - matrix[5] * matrix[8]);
    let loss_report = format!(
        "authority=accepted exact OCCT B-Rep\nformat=Wavefront OBJ\nconversion=exact-body-to-world-space-mesh\neditability_loss=canonical features, rules, and dimensions are not preserved\ntopology_loss=exact topology, analytic surfaces, and durable face identity are not preserved\ntolerance_loss=geometry is approximated by the accepted tessellation under the source tolerance profile\nsource_tolerance={}\nsource_digest={}\n{}\nresult_fingerprint={}\n",
        view.tolerance(),
        view.source_digest(),
        view.producer_identity(),
        view.result_fingerprint()
    );
    let mut mesh_obj = format!(
        "# Ketchup exact body OBJ\n# {}# canonical_authority=canonical source identity and transform\n",
        loss_report.replace('\n', "\n# ")
    );
    for index in 0..view.vertex_count() {
        let [x, y, z] = transform_exact_point(matrix, view.vertex_position_mm(index));
        writeln!(mesh_obj, "v {x:.17} {y:.17} {z:.17}").expect("writing to a String cannot fail");
    }
    let mut current_group = None;
    for triangle_index in 0..view.triangle_count() {
        let group = view.triangle_group_name(triangle_index);
        if current_group.as_ref() != Some(&group) {
            writeln!(mesh_obj, "g {group}").expect("writing to a String cannot fail");
            current_group = Some(group);
        }
        let mut indices = view.triangle_indices(triangle_index);
        if determinant < 0.0 {
            indices.swap(1, 2);
        }
        writeln!(
            mesh_obj,
            "f {} {} {}",
            indices[0] + 1,
            indices[1] + 1,
            indices[2] + 1
        )
        .expect("writing to a String cannot fail");
    }
    ExactMeshExport {
        mesh_obj,
        loss_report,
    }
}

pub fn exact_model_stl_export(
    snapshot: &Snapshot,
    bodies: &[(&ExactBodyPackage, Transform)],
) -> Result<ExactStlExport, ExactProductError> {
    if bodies.is_empty() {
        return Err(ExactProductError::EmptyModelExport);
    }
    if bodies
        .iter()
        .any(|(package, _)| !package.is_current(snapshot))
    {
        return Err(ExactProductError::StaleResult);
    }

    let mut mesh_stl = String::from("solid ketchup_current_model\n");
    let mut facet_count = 0_usize;
    for (package, transform) in bodies {
        let matrix = transform.matrix();
        let determinant = matrix[0] * (matrix[5] * matrix[10] - matrix[6] * matrix[9])
            - matrix[1] * (matrix[4] * matrix[10] - matrix[6] * matrix[8])
            + matrix[2] * (matrix[4] * matrix[9] - matrix[5] * matrix[8]);
        if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
            return Err(ExactProductError::InvalidMeshExport);
        }
        for triangle in package.triangles() {
            let mut indices = triangle.vertex_indices;
            if determinant < 0.0 {
                indices.swap(1, 2);
            }
            let points = indices.map(|index| {
                package
                    .vertices()
                    .get(index as usize)
                    .map(|vertex| transform_exact_point(matrix, vertex.position_mm))
            });
            let [Some(a), Some(b), Some(c)] = points else {
                return Err(ExactProductError::InvalidMeshExport);
            };
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
            if !length.is_finite() || length <= f64::EPSILON {
                return Err(ExactProductError::InvalidMeshExport);
            }
            let normal = [cross[0] / length, cross[1] / length, cross[2] / length];
            writeln!(
                mesh_stl,
                "  facet normal {:.17} {:.17} {:.17}",
                normal[0], normal[1], normal[2]
            )
            .expect("writing to a String cannot fail");
            mesh_stl.push_str("    outer loop\n");
            for point in [a, b, c] {
                writeln!(
                    mesh_stl,
                    "      vertex {:.17} {:.17} {:.17}",
                    point[0], point[1], point[2]
                )
                .expect("writing to a String cannot fail");
            }
            mesh_stl.push_str("    endloop\n  endfacet\n");
            facet_count += 1;
        }
    }
    mesh_stl.push_str("endsolid ketchup_current_model\n");

    let fingerprints = bodies
        .iter()
        .map(|(package, _)| package.result_fingerprint())
        .collect::<Vec<_>>()
        .join(",");
    let loss_report = format!(
        "authority=accepted exact OCCT B-Rep\nformat=ASCII STL\nconversion=current-visible-exact-model-to-world-space-mesh\neditability_loss=canonical features, rules, dimensions, hierarchy, and Undo history are not preserved\ntopology_loss=exact topology, analytic surfaces, assembly identity, and durable face identity are not preserved\ntolerance_loss=geometry is approximated by each accepted tessellation under its source tolerance profile\nsource_digest={}\noccurrence_count={}\nfacet_count={facet_count}\nresult_fingerprints={fingerprints}\n",
        snapshot.canonical_digest(),
        bodies.len(),
    );
    Ok(ExactStlExport {
        mesh_stl,
        loss_report,
    })
}

fn transform_exact_point(matrix: &[f64; 16], point: [f64; 3]) -> [f64; 3] {
    [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
        matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
    ]
}

impl From<ExactRenderPackage> for ExactBodyPackage {
    fn from(package: ExactRenderPackage) -> Self {
        Self::Rectangle(package)
    }
}

impl From<crate::exact_revolve::ExactRevolvePackage> for ExactBodyPackage {
    fn from(package: crate::exact_revolve::ExactRevolvePackage) -> Self {
        Self::Revolve(package)
    }
}

impl From<ExactBRepGraphPackage> for ExactBodyPackage {
    fn from(package: ExactBRepGraphPackage) -> Self {
        Self::Graph(package)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactResultKey {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub producer_feature_id: FeatureId,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
    pub schema: String,
    pub result_fingerprint: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactBodyResultKey {
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub producer_feature_id: FeatureId,
}

#[derive(Clone, Debug)]
pub struct ExactResultRegistry {
    packages: BTreeMap<ExactResultKey, Arc<ExactBodyPackage>>,
    #[cfg(feature = "named-product-fixtures")]
    beam_packages: BTreeMap<BeamExactResultKey, Arc<BeamExactPiecePackage>>,
    contents_stamp: u64,
}

impl Default for ExactResultRegistry {
    fn default() -> Self {
        Self {
            packages: BTreeMap::new(),
            #[cfg(feature = "named-product-fixtures")]
            beam_packages: BTreeMap::new(),
            contents_stamp: next_contents_stamp(),
        }
    }
}

fn next_contents_stamp() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub fn exact_body_terminal_features(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Result<BTreeMap<BodyId, FeatureId>, ExactProductError> {
    let graph = snapshot
        .feature_dependency_graph()
        .map_err(|_| ExactProductError::UnsupportedDefinition)?;
    exact_body_terminal_features_with_graph(snapshot, definition_id, &graph)
}

fn exact_body_terminal_features_with_graph(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    graph: &FeatureDependencyGraph,
) -> Result<BTreeMap<BodyId, FeatureId>, ExactProductError> {
    let definition = snapshot
        .definition(definition_id)
        .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
    let mut terminals = BTreeMap::new();
    for body in definition.bodies() {
        if body.consumed_by().is_some() {
            continue;
        }
        let candidates = definition
            .feature_ids()
            .iter()
            .copied()
            .filter(|feature_id| {
                definition
                    .feature_body_ownership(*feature_id)
                    .and_then(|ownership| ownership.output_body_id())
                    == Some(body.id())
                    && !snapshot.feature_is_suppressed(*feature_id)
                    && snapshot
                        .feature(*feature_id)
                        .is_some_and(|feature| feature.kind().produces_body())
            })
            .filter(|feature_id| {
                graph.dependents(*feature_id).is_some_and(|dependents| {
                    dependents.iter().all(|dependent| {
                        snapshot.feature_is_suppressed(*dependent)
                            || definition
                                .feature_body_ownership(*dependent)
                                .and_then(|ownership| ownership.output_body_id())
                                != Some(body.id())
                            || snapshot
                                .feature(*dependent)
                                .is_none_or(|feature| !feature.kind().produces_body())
                    })
                })
            })
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [] => {}
            [producer_feature_id] => {
                terminals.insert(body.id(), *producer_feature_id);
            }
            _ => {
                return Err(ExactProductError::ConflictingBodyTerminals {
                    definition_id,
                    body_id: body.id(),
                });
            }
        }
    }
    Ok(terminals)
}

fn exact_body_result_key(
    snapshot: &Snapshot,
    package: &ExactBodyPackage,
) -> Result<ExactBodyResultKey, ExactProductError> {
    if !package.is_current(snapshot) {
        return Err(ExactProductError::StaleResult);
    }
    let definition_id = package.definition_id();
    let producer_feature_id = package.producer_feature_id();
    let definition = snapshot
        .definition(definition_id)
        .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
    let body_id = definition
        .feature_body_ownership(producer_feature_id)
        .and_then(|ownership| ownership.output_body_id())
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    if exact_body_terminal_features(snapshot, definition_id)?.get(&body_id)
        != Some(&producer_feature_id)
    {
        return Err(ExactProductError::NonTerminalBodyResult {
            definition_id,
            body_id,
            producer_feature_id,
        });
    }
    Ok(ExactBodyResultKey {
        definition_id,
        body_id,
        producer_feature_id,
    })
}

impl ExactResultRegistry {
    pub fn accept(
        snapshot: &Snapshot,
        packages: impl IntoIterator<Item = Arc<ExactBodyPackage>>,
    ) -> Result<Self, ExactProductError> {
        let mut registry = Self::default();
        for package in packages {
            registry.insert_current(snapshot, package)?;
        }
        Ok(registry)
    }

    #[cfg(feature = "named-product-fixtures")]
    pub fn accept_beam(
        snapshot: &Snapshot,
        packages: impl IntoIterator<Item = Arc<BeamExactPiecePackage>>,
    ) -> Result<Self, ExactProductError> {
        let mut registry = Self::default();
        for package in packages {
            registry.insert_current_beam(snapshot, package)?;
        }
        Ok(registry)
    }

    /// Every product of `previous` whose producer inputs are unchanged in
    /// `snapshot`, rebound to the new revision envelope.
    ///
    /// Derived geometry remains outside canonical state. Rebinding re-derives
    /// the producer request and requires the same dependency-local input digest;
    /// changed branches are dropped while unrelated current branches survive.
    #[must_use]
    pub fn carried_forward(snapshot: &Snapshot, previous: &Self) -> Self {
        let mut registry = Self::default();
        for package in previous.packages.values() {
            let Some(rebound) = package.rebound_to(snapshot) else {
                continue;
            };
            if registry
                .insert_current(snapshot, Arc::new(rebound))
                .is_err()
            {
                continue;
            }
        }
        registry
    }

    pub fn publish_body_results(
        snapshot: &Snapshot,
        previous: &Self,
        packages: impl IntoIterator<Item = Arc<ExactBodyPackage>>,
    ) -> Result<Self, ExactProductError> {
        let mut staged = Self::carried_forward(snapshot, previous);
        let mut occupied = staged
            .body_values(snapshot)?
            .keys()
            .map(|key| (key.definition_id, key.body_id))
            .collect::<BTreeSet<_>>();
        for package in packages {
            let key = exact_body_result_key(snapshot, &package)?;
            if !occupied.insert((key.definition_id, key.body_id)) {
                return Err(ExactProductError::ConflictingBodyPublication {
                    definition_id: key.definition_id,
                    body_id: key.body_id,
                });
            }
            staged.insert_current(snapshot, package)?;
        }
        Ok(staged)
    }

    pub fn body_values<'a>(
        &'a self,
        snapshot: &Snapshot,
    ) -> Result<BTreeMap<ExactBodyResultKey, &'a Arc<ExactBodyPackage>>, ExactProductError> {
        let mut values = BTreeMap::new();
        let mut occupied = BTreeSet::new();
        let document_id = snapshot.document_id();
        let source_revision = snapshot.revision_id();
        let source_digest = snapshot.canonical_digest();
        let graph = snapshot
            .feature_dependency_graph()
            .map_err(|_| ExactProductError::UnsupportedDefinition)?;
        for (result_key, package) in &self.packages {
            if result_key.document_id != document_id
                || result_key.source_revision != source_revision
                || result_key.source_digest != source_digest
            {
                continue;
            }
            let definition_id = package.definition_id();
            let producer_feature_id = package.producer_feature_id();
            let definition = snapshot
                .definition(definition_id)
                .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
            let body_id = definition
                .feature_body_ownership(producer_feature_id)
                .and_then(|ownership| ownership.output_body_id())
                .ok_or(ExactProductError::InvalidWorkerEvidence)?;
            if exact_body_terminal_features_with_graph(snapshot, definition_id, &graph)?
                .get(&body_id)
                != Some(&producer_feature_id)
            {
                continue;
            }
            if !occupied.insert((definition_id, body_id)) {
                return Err(ExactProductError::ConflictingBodyPublication {
                    definition_id,
                    body_id,
                });
            }
            values.insert(
                ExactBodyResultKey {
                    definition_id,
                    body_id,
                    producer_feature_id,
                },
                package,
            );
        }
        Ok(values)
    }

    pub fn get_body(
        &self,
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        body_id: BodyId,
    ) -> Result<Option<&Arc<ExactBodyPackage>>, ExactProductError> {
        Ok(self
            .body_values(snapshot)?
            .into_iter()
            .find_map(|(key, package)| {
                (key.definition_id == definition_id && key.body_id == body_id).then_some(package)
            }))
    }

    pub fn insert_current(
        &mut self,
        snapshot: &Snapshot,
        package: Arc<ExactBodyPackage>,
    ) -> Result<(), ExactProductError> {
        if !package.is_current(snapshot) {
            return Err(ExactProductError::StaleResult);
        }
        let key = package.result_key();
        if self.packages.contains_key(&key) {
            return Err(ExactProductError::DuplicateResult {
                definition_id: key.definition_id,
                producer_feature_id: key.producer_feature_id,
            });
        }
        self.packages.insert(key, package);
        self.contents_stamp = next_contents_stamp();
        Ok(())
    }

    #[cfg(feature = "named-product-fixtures")]
    pub fn insert_current_beam(
        &mut self,
        snapshot: &Snapshot,
        package: Arc<BeamExactPiecePackage>,
    ) -> Result<(), ExactProductError> {
        if package.identity.document_id != snapshot.document_id()
            || package.identity.source_revision != snapshot.revision_id()
            || package.identity.source_digest != snapshot.canonical_digest()
        {
            return Err(ExactProductError::StaleResult);
        }
        if !package.has_valid_registry_evidence() {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let key = package.result_key();
        if self
            .beam_packages
            .keys()
            .any(|existing| existing.piece == key.piece)
        {
            return Err(ExactProductError::DuplicateDerivedResult {
                piece: key.piece.clone(),
            });
        }
        self.beam_packages.insert(key, package);
        self.contents_stamp = next_contents_stamp();
        Ok(())
    }

    /// A process-unique stamp of what this registry currently holds.
    ///
    /// Products are rebound to a new revision without changing how many there
    /// are, so a consumer that caches derived work cannot invalidate on the
    /// package count. Comparing this integer is exact and costs nothing in a
    /// hot path, unlike re-deriving every package's freshness per frame.
    #[must_use]
    pub const fn contents_stamp(&self) -> u64 {
        self.contents_stamp
    }

    #[must_use]
    pub fn get_result(&self, key: &ExactResultKey) -> Option<&Arc<ExactBodyPackage>> {
        self.packages.get(key)
    }

    #[must_use]
    #[cfg(feature = "named-product-fixtures")]
    pub fn get_beam_result(&self, key: &BeamExactResultKey) -> Option<&Arc<BeamExactPiecePackage>> {
        self.beam_packages.get(key)
    }

    #[must_use]
    #[cfg(feature = "named-product-fixtures")]
    pub fn get_beam(&self, piece: &DerivedIdentity) -> Option<&Arc<BeamExactPiecePackage>> {
        let mut matches = self
            .beam_packages
            .iter()
            .filter(|(key, _)| key.piece == *piece)
            .map(|(_, package)| package);
        let package = matches.next()?;
        matches.next().is_none().then_some(package)
    }

    #[must_use]
    pub fn get(&self, definition_id: &DefinitionId) -> Option<&Arc<ExactBodyPackage>> {
        let mut matches = self
            .packages
            .iter()
            .filter(|(key, _)| key.definition_id == *definition_id)
            .map(|(_, package)| package);
        let package = matches.next()?;
        matches.next().is_none().then_some(package)
    }

    pub fn render_values<'a>(
        &'a self,
        snapshot: &'a Snapshot,
    ) -> impl Iterator<Item = &'a Arc<ExactBodyPackage>> {
        self.body_values(snapshot)
            .unwrap_or_default()
            .into_iter()
            .filter_map(move |(key, package)| {
                snapshot
                    .definition(key.definition_id)
                    .and_then(|definition| definition.body(key.body_id))
                    .is_some_and(|body| body.visible())
                    .then_some(package)
            })
    }

    #[must_use]
    pub fn render_by_definition<'a>(
        &'a self,
        snapshot: &'a Snapshot,
    ) -> BTreeMap<DefinitionId, &'a Arc<ExactBodyPackage>> {
        let mut matches = BTreeMap::new();
        for package in self.render_values(snapshot) {
            matches
                .entry(package.definition_id())
                .and_modify(|candidate| *candidate = None)
                .or_insert(Some(package));
        }
        matches
            .into_iter()
            .filter_map(|(definition_id, package)| package.map(|package| (definition_id, package)))
            .collect()
    }

    #[must_use]
    pub fn get_render<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        definition_id: DefinitionId,
    ) -> Option<&'a Arc<ExactBodyPackage>> {
        let mut matches = self
            .render_values(snapshot)
            .filter(|package| package.definition_id() == definition_id);
        let package = matches.next()?;
        matches.next().is_none().then_some(package)
    }

    #[must_use]
    pub fn resolve_topological_reference(
        &self,
        snapshot: &Snapshot,
        reference: &TopologicalElementRef,
    ) -> TopologicalReferenceResolution {
        let candidates = self
            .packages
            .values()
            .filter(|package| package.is_current(snapshot))
            .flat_map(|package| package.topological_references());
        resolve_role_neutral_topological_reference(snapshot, reference, candidates)
    }

    #[must_use]
    pub fn resolve_reference(
        &self,
        snapshot: &Snapshot,
        reference: &BodySubshapeRef,
    ) -> ExactReferenceResolution {
        if !reference.has_valid_lineage() {
            return ExactReferenceResolution::Quarantined {
                reason: ExactReferenceQuarantineReason::InvalidLineage,
            };
        }
        if reference.document_id != snapshot.document_id() {
            return ExactReferenceResolution::Quarantined {
                reason: ExactReferenceQuarantineReason::WrongDocument,
            };
        }
        if snapshot
            .feature(reference.producer_feature_id)
            .is_none_or(|producer| producer.definition_id() != reference.definition_id)
        {
            return ExactReferenceResolution::Lost;
        }

        let candidates = self
            .packages
            .values()
            .filter(|package| package.is_current(snapshot))
            .flat_map(|package| package.references())
            .filter(|candidate| {
                candidate.has_valid_lineage()
                    && candidate.document_id == reference.document_id
                    && candidate.definition_id == reference.definition_id
                    && candidate.profile_feature_id == reference.profile_feature_id
                    && candidate.producer_feature_id == reference.producer_feature_id
                    && candidate.semantic_role == reference.semantic_role
                    && candidate.source_element_id == reference.source_element_id
                    && candidate.expected_type == reference.expected_type
                    && candidate.expected_cardinality == reference.expected_cardinality
                    && candidate.stability == reference.stability
            })
            .collect::<Vec<_>>();
        let [candidate] = candidates.as_slice() else {
            return if candidates.is_empty() {
                ExactReferenceResolution::Lost
            } else {
                ExactReferenceResolution::Ambiguous {
                    candidate_count: candidates.len(),
                }
            };
        };
        if candidate.lineage_digest != reference.lineage_digest {
            return ExactReferenceResolution::Quarantined {
                reason: ExactReferenceQuarantineReason::InvalidLineage,
            };
        }
        if candidate.evaluator != reference.evaluator
            || candidate.backend != reference.backend
            || candidate.tolerance != reference.tolerance
        {
            return ExactReferenceResolution::Quarantined {
                reason: ExactReferenceQuarantineReason::IncompatibleEvaluationEnvelope,
            };
        }
        ExactReferenceResolution::Resolved {
            reference: Box::new((*candidate).clone()),
        }
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<ExactBodyPackage>> {
        self.packages.values()
    }

    #[cfg(feature = "named-product-fixtures")]
    pub fn beam_values(&self) -> impl Iterator<Item = &Arc<BeamExactPiecePackage>> {
        self.beam_packages.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    #[must_use]
    #[cfg(feature = "named-product-fixtures")]
    pub fn beam_len(&self) -> usize {
        self.beam_packages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Whether every product held here is already bound to `snapshot`.
    ///
    /// Products are keyed by the source they were accepted for, so this reads
    /// their keys instead of revalidating their evidence, which makes it cheap
    /// enough to ask once per painted frame.
    #[must_use]
    pub fn is_bound_to(&self, snapshot: &Snapshot) -> bool {
        self.packages.keys().all(|key| {
            key.document_id == snapshot.document_id()
                && key.source_revision == snapshot.revision_id()
                && key.source_digest == snapshot.canonical_digest()
        })
    }

    pub fn clear(&mut self) {
        self.packages.clear();
        #[cfg(feature = "named-product-fixtures")]
        self.beam_packages.clear();
        self.contents_stamp = next_contents_stamp();
    }
}

impl ExactRenderPackage {
    #[must_use]
    pub fn reference(&self, role: ExactFaceRole) -> Option<&BodySubshapeRef> {
        self.references
            .iter()
            .find(|reference| reference.role() == Some(role))
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.document_id == snapshot.document_id()
            && self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && ExactFeatureChainRequest::from_snapshot_for_producer(
                snapshot,
                self.identity.definition_id,
                self.identity.producer_feature_id,
            )
            .is_ok_and(|request| self.validate_for_request(&request).is_ok())
    }

    pub fn validate_for_request(
        &self,
        request: &ExactFeatureChainRequest,
    ) -> Result<(), ExactProductError> {
        let expected_roles = request.expected_face_roles();
        let mut actual_roles = self
            .references
            .iter()
            .map(BodySubshapeRef::role)
            .collect::<Option<Vec<_>>>()
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        actual_roles.sort_unstable();
        let mut sorted_expected = expected_roles.to_vec();
        sorted_expected.sort_unstable();
        let is_cut = request
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.operation == BooleanOperation::Cut);
        let is_pocket = request.pocket_depth_bits.is_some();
        let is_circle = request.circle.is_some()
            || request.boolean.as_ref().is_some_and(|boolean| {
                matches!(
                    boolean.operation,
                    BooleanOperation::Union | BooleanOperation::Intersect
                ) && boolean.circle.is_some()
            });
        let is_mixed = request.mixed_profile.is_some();
        let is_polygon_cut = request
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.profile.is_some());
        let mixed_mesh = (is_mixed || is_polygon_cut)
            .then(|| render_mesh(request))
            .transpose()?
            .map(|(vertices, triangles)| (vertices.len(), triangles));
        let is_circular_cut = request.boolean.as_ref().is_some_and(|boolean| {
            boolean.operation == BooleanOperation::Cut && boolean.circle.is_some()
        });
        let [width, depth, _] = request.dimensions_mm();
        let circular_side_mesh = request
            .boolean
            .as_ref()
            .filter(|boolean| {
                matches!(
                    boolean.operation,
                    BooleanOperation::Cut
                        | BooleanOperation::Union
                        | BooleanOperation::Intersect
                        | BooleanOperation::Split
                ) && boolean.circle.is_some_and(|circle| {
                    circle.side_overlap(width, depth).is_some()
                        || circle.corner_overlap(width, depth).is_some()
                        || matches!(
                            boolean.operation,
                            BooleanOperation::Cut
                                | BooleanOperation::Union
                                | BooleanOperation::Intersect
                                | BooleanOperation::Split
                        ) && circle.outside_side_overlap(width, depth).is_some()
                        || matches!(
                            boolean.operation,
                            BooleanOperation::Cut
                                | BooleanOperation::Union
                                | BooleanOperation::Intersect
                                | BooleanOperation::Split
                        ) && circle.center_on_side_overlap(width, depth).is_some()
                        || matches!(
                            boolean.operation,
                            BooleanOperation::Cut
                                | BooleanOperation::Union
                                | BooleanOperation::Intersect
                                | BooleanOperation::Split
                        ) && circle.center_on_corner_overlap(width, depth).is_some()
                        || matches!(
                            boolean.operation,
                            BooleanOperation::Cut
                                | BooleanOperation::Union
                                | BooleanOperation::Intersect
                                | BooleanOperation::Split
                        ) && circle.outside_corner_overlap(width, depth).is_some()
                })
            })
            .map(|_| render_mesh(request))
            .transpose()?
            .map(|(vertices, triangles)| (vertices.len(), triangles));
        let expected_counts = if let Some((vertex_count, triangles)) = &circular_side_mesh {
            (*vertex_count, triangles.len())
        } else if is_circular_cut {
            (128, 256)
        } else if is_circle {
            (66, 128)
        } else if let Some((vertex_count, triangles)) = &mixed_mesh {
            (*vertex_count, triangles.len())
        } else if is_pocket {
            (16, 28)
        } else if is_cut {
            (16, 32)
        } else {
            (8, 12)
        };
        let expected_triangle_roles = expected_roles.iter().all(|role| {
            let actual = self
                .triangles
                .iter()
                .filter(|triangle| triangle.face_role == Some(*role))
                .count();
            if let Some((_, expected_triangles)) = &circular_side_mesh {
                actual
                    == expected_triangles
                        .iter()
                        .filter(|triangle| triangle.face_role == Some(*role))
                        .count()
            } else if is_circular_cut {
                match role {
                    ExactFaceRole::Top | ExactFaceRole::Bottom | ExactFaceRole::CutCircle => {
                        actual == 64
                    }
                    ExactFaceRole::East => actual > 0,
                    _ => false,
                }
            } else if let Some((_, expected_triangles)) = &mixed_mesh {
                actual
                    == expected_triangles
                        .iter()
                        .filter(|triangle| triangle.face_role == Some(*role))
                        .count()
            } else if is_circle {
                match role {
                    ExactFaceRole::Top | ExactFaceRole::Bottom => actual == 32,
                    ExactFaceRole::CircleSide => actual == 64,
                    _ => false,
                }
            } else {
                let expected = match role {
                    ExactFaceRole::Top if is_cut => 8,
                    ExactFaceRole::Bottom if is_cut && !is_pocket => 8,
                    _ => 2,
                };
                actual == expected
            }
        });
        if self.identity.schema != EXACT_PRODUCT_SCHEMA_V1
            || self.identity.document_id != request.document_id
            || self.identity.source_revision != request.source_revision
            || self.identity.source_digest != request.source_digest
            || self.identity.definition_id != request.definition_id
            || self.identity.profile_feature_id != request.profile_feature_id
            || self.identity.extrusion_feature_id != request.extrusion_feature_id
            || self.identity.producer_feature_id != request.producer_feature_id()
            || self.identity.canonical_input_digest != request.canonical_input_digest
            || self.identity.evaluator != request.evaluator()
            || self.identity.exact_input_digest.is_empty()
            || self.identity.result_fingerprint.is_empty()
            || self.identity.backend.is_empty()
            || self.identity.tolerance.is_empty()
            || self.vertices.len() != expected_counts.0
            || self.triangles.len() != expected_counts.1
            || self.triangles.iter().any(|triangle| {
                triangle
                    .vertex_indices
                    .iter()
                    .any(|index| *index as usize >= self.vertices.len())
            })
            || self.references.len() != expected_roles.len()
            || actual_roles != sorted_expected
            || !expected_triangle_roles
            || self.references.iter().any(|reference| {
                !reference.matches_request(request)
                    || reference.exact_input_digest != self.identity.exact_input_digest
                    || reference.result_fingerprint != self.identity.result_fingerprint
                    || reference.backend != self.identity.backend
                    || reference.tolerance != self.identity.tolerance
            })
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssemblySelectionTarget {
    pub instance_path: InstancePath,
    pub body: BodySubshapeRef,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactCircleProfile {
    pub center_x_bits: u64,
    pub center_y_bits: u64,
    pub radius_bits: u64,
    pub clockwise: bool,
}

impl ExactCircleProfile {
    #[must_use]
    pub fn side_overlap(self, width: f64, depth: f64) -> Option<(f64, [f64; 4])> {
        let center_x = f64::from_bits(self.center_x_bits);
        let center_y = f64::from_bits(self.center_y_bits);
        let radius = f64::from_bits(self.radius_bits);
        if [center_x, center_y, radius, width, depth]
            .into_iter()
            .any(|value| !value.is_finite())
            || radius <= 0.0
            || width <= 0.0
            || depth <= 0.0
            || center_x <= 1.0e-6
            || center_x >= width - 1.0e-6
            || center_y <= 1.0e-6
            || center_y >= depth - 1.0e-6
        {
            return None;
        }
        let bounds = [
            center_x - radius,
            center_y - radius,
            center_x + radius,
            center_y + radius,
        ];
        let crossed = [
            bounds[0] < -1.0e-6,
            bounds[2] > width + 1.0e-6,
            bounds[1] < -1.0e-6,
            bounds[3] > depth + 1.0e-6,
        ];
        if crossed.into_iter().filter(|crossed| *crossed).count() != 1
            || (crossed[0] || crossed[1]) && (bounds[1] <= 1.0e-6 || bounds[3] >= depth - 1.0e-6)
            || (crossed[2] || crossed[3]) && (bounds[0] <= 1.0e-6 || bounds[2] >= width - 1.0e-6)
        {
            return None;
        }
        let distance = if crossed[0] {
            center_x
        } else if crossed[1] {
            width - center_x
        } else if crossed[2] {
            center_y
        } else {
            depth - center_y
        };
        let chord_half = (radius * radius - distance * distance).sqrt();
        let outside_area = radius * radius * (distance / radius).acos() - distance * chord_half;
        let overlap_area = std::f64::consts::PI * radius * radius - outside_area;
        let clipped_bounds = [
            bounds[0].max(0.0),
            bounds[1].max(0.0),
            bounds[2].min(width),
            bounds[3].min(depth),
        ];
        (overlap_area.is_finite() && overlap_area > 1.0e-9)
            .then_some((overlap_area, clipped_bounds))
    }

    #[must_use]
    pub fn center_on_side_overlap(self, width: f64, depth: f64) -> Option<(f64, [f64; 4])> {
        let center_x = f64::from_bits(self.center_x_bits);
        let center_y = f64::from_bits(self.center_y_bits);
        let radius = f64::from_bits(self.radius_bits);
        if [center_x, center_y, radius, width, depth]
            .into_iter()
            .any(|value| !value.is_finite())
            || radius <= 0.0
            || width <= 0.0
            || depth <= 0.0
        {
            return None;
        }
        let centered_on = [
            center_x.abs() <= 1.0e-6,
            (center_x - width).abs() <= 1.0e-6,
            center_y.abs() <= 1.0e-6,
            (center_y - depth).abs() <= 1.0e-6,
        ];
        if centered_on.into_iter().filter(|centered| *centered).count() != 1
            || (centered_on[0] || centered_on[1])
                && (center_y - radius <= 1.0e-6 || center_y + radius >= depth - 1.0e-6)
            || (centered_on[2] || centered_on[3])
                && (center_x - radius <= 1.0e-6 || center_x + radius >= width - 1.0e-6)
        {
            return None;
        }
        let bounds = [
            (center_x - radius).max(0.0),
            (center_y - radius).max(0.0),
            (center_x + radius).min(width),
            (center_y + radius).min(depth),
        ];
        Some((0.5 * std::f64::consts::PI * radius * radius, bounds))
    }

    #[must_use]
    pub fn center_on_corner_overlap(self, width: f64, depth: f64) -> Option<(f64, [f64; 4])> {
        let center_x = f64::from_bits(self.center_x_bits);
        let center_y = f64::from_bits(self.center_y_bits);
        let radius = f64::from_bits(self.radius_bits);
        if [center_x, center_y, radius, width, depth]
            .into_iter()
            .any(|value| !value.is_finite())
            || radius <= 0.0
            || width <= 0.0
            || depth <= 0.0
        {
            return None;
        }
        let centered_on_x = [center_x.abs() <= 1.0e-6, (center_x - width).abs() <= 1.0e-6];
        let centered_on_y = [center_y.abs() <= 1.0e-6, (center_y - depth).abs() <= 1.0e-6];
        if centered_on_x
            .into_iter()
            .filter(|centered| *centered)
            .count()
            != 1
            || centered_on_y
                .into_iter()
                .filter(|centered| *centered)
                .count()
                != 1
            || radius >= width - 1.0e-6
            || radius >= depth - 1.0e-6
        {
            return None;
        }
        let bounds = [
            (center_x - radius).max(0.0),
            (center_y - radius).max(0.0),
            (center_x + radius).min(width),
            (center_y + radius).min(depth),
        ];
        Some((0.25 * std::f64::consts::PI * radius * radius, bounds))
    }

    #[must_use]
    pub fn outside_side_overlap(self, width: f64, depth: f64) -> Option<(f64, [f64; 4])> {
        let center_x = f64::from_bits(self.center_x_bits);
        let center_y = f64::from_bits(self.center_y_bits);
        let radius = f64::from_bits(self.radius_bits);
        if [center_x, center_y, radius, width, depth]
            .into_iter()
            .any(|value| !value.is_finite())
            || radius <= 0.0
            || width <= 0.0
            || depth <= 0.0
        {
            return None;
        }
        let bounds = [
            center_x - radius,
            center_y - radius,
            center_x + radius,
            center_y + radius,
        ];
        let outside = [
            center_x < -1.0e-6,
            center_x > width + 1.0e-6,
            center_y < -1.0e-6,
            center_y > depth + 1.0e-6,
        ];
        if outside.into_iter().filter(|outside| *outside).count() != 1
            || (outside[0] || outside[1]) && (bounds[1] <= 1.0e-6 || bounds[3] >= depth - 1.0e-6)
            || (outside[2] || outside[3]) && (bounds[0] <= 1.0e-6 || bounds[2] >= width - 1.0e-6)
        {
            return None;
        }
        let distance = if outside[0] {
            -center_x
        } else if outside[1] {
            center_x - width
        } else if outside[2] {
            -center_y
        } else {
            center_y - depth
        };
        if distance >= radius - 1.0e-6 {
            return None;
        }
        let chord_half = (radius * radius - distance * distance).sqrt();
        let overlap_area = radius * radius * (distance / radius).acos() - distance * chord_half;
        let clipped_bounds = if outside[0] || outside[1] {
            [
                bounds[0].max(0.0),
                center_y - chord_half,
                bounds[2].min(width),
                center_y + chord_half,
            ]
        } else {
            [
                center_x - chord_half,
                bounds[1].max(0.0),
                center_x + chord_half,
                bounds[3].min(depth),
            ]
        };
        (overlap_area.is_finite() && overlap_area > 1.0e-9)
            .then_some((overlap_area, clipped_bounds))
    }

    #[must_use]
    pub fn outside_corner_overlap(self, width: f64, depth: f64) -> Option<(f64, [f64; 4])> {
        let center_x = f64::from_bits(self.center_x_bits);
        let center_y = f64::from_bits(self.center_y_bits);
        let radius = f64::from_bits(self.radius_bits);
        if [center_x, center_y, radius, width, depth]
            .into_iter()
            .any(|value| !value.is_finite())
            || radius <= 0.0
            || width <= 0.0
            || depth <= 0.0
        {
            return None;
        }
        let outside_x = [center_x < -1.0e-6, center_x > width + 1.0e-6];
        let outside_y = [center_y < -1.0e-6, center_y > depth + 1.0e-6];
        if outside_x.into_iter().filter(|outside| *outside).count() != 1
            || outside_y.into_iter().filter(|outside| *outside).count() != 1
        {
            return None;
        }
        let distance_x = if outside_x[0] {
            -center_x
        } else {
            center_x - width
        };
        let distance_y = if outside_y[0] {
            -center_y
        } else {
            center_y - depth
        };
        let bounds = [
            center_x - radius,
            center_y - radius,
            center_x + radius,
            center_y + radius,
        ];
        if distance_x >= radius - 1.0e-6
            || distance_y >= radius - 1.0e-6
            || distance_x * distance_x + distance_y * distance_y >= radius * radius - 1.0e-9
            || outside_x[0] && bounds[2] >= width - 1.0e-6
            || outside_x[1] && bounds[0] <= 1.0e-6
            || outside_y[0] && bounds[3] >= depth - 1.0e-6
            || outside_y[1] && bounds[1] <= 1.0e-6
        {
            return None;
        }
        let limit = (radius * radius - distance_y * distance_y).sqrt();
        let primitive = |value: f64| {
            0.5 * (value * (radius * radius - value * value).sqrt()
                + radius * radius * (value / radius).asin())
        };
        let overlap_area =
            primitive(limit) - primitive(distance_x) - distance_y * (limit - distance_x);
        let limit_y = (radius * radius - distance_x * distance_x).sqrt();
        let clipped_bounds = [
            if outside_x[0] { 0.0 } else { center_x - limit },
            if outside_y[0] {
                0.0
            } else {
                center_y - limit_y
            },
            if outside_x[0] {
                center_x + limit
            } else {
                width
            },
            if outside_y[0] {
                center_y + limit_y
            } else {
                depth
            },
        ];
        (overlap_area.is_finite() && overlap_area > 1.0e-9)
            .then_some((overlap_area, clipped_bounds))
    }

    #[must_use]
    pub fn corner_overlap(self, width: f64, depth: f64) -> Option<(f64, [f64; 4])> {
        let center_x = f64::from_bits(self.center_x_bits);
        let center_y = f64::from_bits(self.center_y_bits);
        let radius = f64::from_bits(self.radius_bits);
        if [center_x, center_y, radius, width, depth]
            .into_iter()
            .any(|value| !value.is_finite())
            || radius <= 0.0
            || width <= 0.0
            || depth <= 0.0
            || center_x <= 1.0e-6
            || center_x >= width - 1.0e-6
            || center_y <= 1.0e-6
            || center_y >= depth - 1.0e-6
        {
            return None;
        }
        let bounds = [
            center_x - radius,
            center_y - radius,
            center_x + radius,
            center_y + radius,
        ];
        let crossed = [
            bounds[0] < -1.0e-6,
            bounds[2] > width + 1.0e-6,
            bounds[1] < -1.0e-6,
            bounds[3] > depth + 1.0e-6,
        ];
        if crossed.into_iter().filter(|crossed| *crossed).count() != 2
            || crossed[0] && crossed[1]
            || crossed[2] && crossed[3]
        {
            return None;
        }
        let distance_x = if crossed[0] {
            center_x
        } else {
            width - center_x
        };
        let distance_y = if crossed[2] {
            center_y
        } else {
            depth - center_y
        };
        if distance_x * distance_x + distance_y * distance_y >= radius * radius - 1.0e-9 {
            return None;
        }
        let cap = |distance: f64| {
            radius * radius * (distance / radius).acos()
                - distance * (radius * radius - distance * distance).sqrt()
        };
        let primitive = |value: f64| {
            0.5 * (value * (radius * radius - value * value).sqrt()
                + radius * radius * (value / radius).asin())
        };
        let limit = (radius * radius - distance_y * distance_y).sqrt();
        let shared_outside =
            primitive(limit) - distance_y * limit - primitive(distance_x) + distance_y * distance_x;
        let overlap_area =
            std::f64::consts::PI * radius * radius - cap(distance_x) - cap(distance_y)
                + shared_outside;
        let clipped_bounds = [
            bounds[0].max(0.0),
            bounds[1].max(0.0),
            bounds[2].min(width),
            bounds[3].min(depth),
        ];
        (overlap_area.is_finite() && overlap_area > 1.0e-9)
            .then_some((overlap_area, clipped_bounds))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactProfileSegment {
    Line {
        start_bits: [u64; 2],
        end_bits: [u64; 2],
    },
    CircularArc {
        start_bits: [u64; 2],
        end_bits: [u64; 2],
        center_bits: [u64; 2],
        clockwise: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactMixedProfile {
    pub segments: Vec<ExactProfileSegment>,
    pub bounds_bits: [u64; 4],
    pub area_bits: u64,
}

impl ExactMixedProfile {
    #[must_use]
    pub fn has_only_line_segments(&self) -> bool {
        self.segments
            .iter()
            .all(|segment| matches!(segment, ExactProfileSegment::Line { .. }))
    }

    #[must_use]
    pub fn is_strict_convex_line_arc_profile(&self) -> bool {
        if !self
            .segments
            .iter()
            .any(|segment| matches!(segment, ExactProfileSegment::Line { .. }))
            || !self
                .segments
                .iter()
                .any(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
        {
            return false;
        }
        let scale = self
            .bounds_bits
            .map(f64::from_bits)
            .into_iter()
            .map(f64::abs)
            .fold(1.0, f64::max);
        let tolerance = scale * scale * 1.0e-9;
        let mut boundary = Vec::new();
        for segment in &self.segments {
            match segment {
                ExactProfileSegment::Line { start_bits, .. } => {
                    boundary.push(start_bits.map(f64::from_bits));
                }
                ExactProfileSegment::CircularArc {
                    start_bits,
                    end_bits,
                    center_bits,
                    clockwise,
                } => {
                    let start = start_bits.map(f64::from_bits);
                    let end = end_bits.map(f64::from_bits);
                    let center = center_bits.map(f64::from_bits);
                    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                    let Some(sweep) = directed_arc_sweep(start_angle, end_angle, *clockwise) else {
                        return false;
                    };
                    if sweep.abs() > std::f64::consts::PI + 1.0e-9 {
                        return false;
                    }
                    let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                    let steps = (sweep.abs() / (std::f64::consts::PI / 16.0))
                        .ceil()
                        .max(1.0) as usize;
                    for step in 0..steps {
                        let angle = start_angle + sweep * step as f64 / steps as f64;
                        boundary.push([
                            center[0] + radius * angle.cos(),
                            center[1] + radius * angle.sin(),
                        ]);
                    }
                }
            }
        }
        if boundary.len() < 3 {
            return false;
        }
        for left in 0..boundary.len() {
            let left_next = (left + 1) % boundary.len();
            for right in (left + 1)..boundary.len() {
                let right_next = (right + 1) % boundary.len();
                if left == right_next || left_next == right {
                    continue;
                }
                if planar_line_segments_intersect(
                    boundary[left],
                    boundary[left_next],
                    boundary[right],
                    boundary[right_next],
                ) {
                    return false;
                }
            }
        }
        let mut orientation = 0_i8;
        for index in 0..boundary.len() {
            let previous = boundary[index];
            let current = boundary[(index + 1) % boundary.len()];
            let next = boundary[(index + 2) % boundary.len()];
            let cross = (current[0] - previous[0]) * (next[1] - current[1])
                - (current[1] - previous[1]) * (next[0] - current[0]);
            if cross.abs() <= tolerance {
                continue;
            }
            let turn = if cross > 0.0 { 1 } else { -1 };
            if orientation != 0 && orientation != turn {
                return false;
            }
            orientation = turn;
        }
        orientation != 0
            && self.segments.iter().all(|segment| match segment {
                ExactProfileSegment::Line { .. } => true,
                ExactProfileSegment::CircularArc {
                    start_bits,
                    end_bits,
                    center_bits,
                    clockwise,
                } => {
                    let start = start_bits.map(f64::from_bits);
                    let end = end_bits.map(f64::from_bits);
                    let center = center_bits.map(f64::from_bits);
                    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                    directed_arc_sweep(start_angle, end_angle, *clockwise)
                        .is_some_and(|sweep| sweep.signum() == f64::from(orientation))
                }
            })
    }

    #[must_use]
    pub fn strict_convex_line_clipped_side_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        if !self.is_strict_convex_line_arc_profile()
            || self.segments.len() != 5
            || self
                .segments
                .iter()
                .filter(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
                .count()
                != 1
            || !width.is_finite()
            || !depth.is_finite()
            || width <= 0.0
            || depth <= 0.0
        {
            return None;
        }
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let tolerance = 1.0e-6;
        let candidates = [
            (
                min_x < -tolerance
                    && max_x > tolerance
                    && max_x < width - tolerance
                    && min_y > tolerance
                    && max_y < depth - tolerance,
                0_usize,
                min_x,
                0.0,
                true,
                min_y,
                max_y,
                [0.0, min_y, max_x, max_y],
            ),
            (
                min_x > tolerance
                    && min_x < width - tolerance
                    && max_x > width + tolerance
                    && min_y > tolerance
                    && max_y < depth - tolerance,
                0,
                max_x,
                width,
                false,
                min_y,
                max_y,
                [min_x, min_y, width, max_y],
            ),
            (
                min_y < -tolerance
                    && max_y > tolerance
                    && max_y < depth - tolerance
                    && min_x > tolerance
                    && max_x < width - tolerance,
                1,
                min_y,
                0.0,
                true,
                min_x,
                max_x,
                [min_x, 0.0, max_x, max_y],
            ),
            (
                min_y > tolerance
                    && min_y < depth - tolerance
                    && max_y > depth + tolerance
                    && min_x > tolerance
                    && max_x < width - tolerance,
                1,
                max_y,
                depth,
                false,
                min_x,
                max_x,
                [min_x, min_y, max_x, depth],
            ),
        ];
        let mut candidates = candidates.into_iter().filter(|candidate| candidate.0);
        let (_, axis, outer, limit, keep_greater, orthogonal_min, orthogonal_max, bounds) =
            candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        let orthogonal_axis = 1 - axis;
        let inside = |value: f64| {
            if keep_greater {
                value > limit + tolerance
            } else {
                value < limit - tolerance
            }
        };
        let same = |left: f64, right: f64| (left - right).abs() <= tolerance;
        let mut outer_lines = 0;
        let mut connector_lines = 0;
        for segment in &self.segments {
            match segment {
                ExactProfileSegment::Line {
                    start_bits,
                    end_bits,
                } => {
                    let start = start_bits.map(f64::from_bits);
                    let end = end_bits.map(f64::from_bits);
                    let outer_line = same(start[axis], outer)
                        && same(end[axis], outer)
                        && ((same(start[orthogonal_axis], orthogonal_min)
                            && same(end[orthogonal_axis], orthogonal_max))
                            || (same(start[orthogonal_axis], orthogonal_max)
                                && same(end[orthogonal_axis], orthogonal_min)));
                    if outer_line {
                        outer_lines += 1;
                        continue;
                    }
                    let connector = same(start[orthogonal_axis], end[orthogonal_axis])
                        && (same(start[orthogonal_axis], orthogonal_min)
                            || same(start[orthogonal_axis], orthogonal_max))
                        && ((same(start[axis], outer) && inside(end[axis]))
                            || (same(end[axis], outer) && inside(start[axis])));
                    if connector {
                        connector_lines += 1;
                    } else if !inside(start[axis]) || !inside(end[axis]) {
                        return None;
                    }
                }
                ExactProfileSegment::CircularArc {
                    start_bits,
                    end_bits,
                    center_bits,
                    clockwise,
                } => {
                    let start = start_bits.map(f64::from_bits);
                    let end = end_bits.map(f64::from_bits);
                    let center = center_bits.map(f64::from_bits);
                    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                    let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)?;
                    let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                    let mut points = vec![start, end];
                    for angle in [
                        0.0,
                        std::f64::consts::FRAC_PI_2,
                        std::f64::consts::PI,
                        3.0 * std::f64::consts::FRAC_PI_2,
                    ] {
                        if angle_on_directed_arc(start_angle, sweep, angle) {
                            points.push([
                                center[0] + radius * angle.cos(),
                                center[1] + radius * angle.sin(),
                            ]);
                        }
                    }
                    if points.into_iter().any(|point| !inside(point[axis])) {
                        return None;
                    }
                }
            }
        }
        if outer_lines != 1 || connector_lines != 2 {
            return None;
        }
        let outside_area = (limit - outer).abs() * (orthogonal_max - orthogonal_min);
        let overlap_area = f64::from_bits(self.area_bits) - outside_area;
        (overlap_area > tolerance).then_some((overlap_area, bounds))
    }

    #[must_use]
    pub fn strict_convex_arc_only_clipped_side_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        if !self.is_strict_convex_line_arc_profile()
            || self.segments.len() != 5
            || self
                .segments
                .iter()
                .filter(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
                .count()
                != 1
            || !width.is_finite()
            || !depth.is_finite()
            || width <= 0.0
            || depth <= 0.0
        {
            return None;
        }
        let tolerance = 1.0e-6;
        let point_inside = |point: [f64; 2]| {
            point[0] > tolerance
                && point[0] < width - tolerance
                && point[1] > tolerance
                && point[1] < depth - tolerance
        };
        if self.segments.iter().any(|segment| match segment {
            ExactProfileSegment::Line {
                start_bits,
                end_bits,
            }
            | ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                ..
            } => {
                !point_inside(start_bits.map(f64::from_bits))
                    || !point_inside(end_bits.map(f64::from_bits))
            }
        }) {
            return None;
        }

        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let candidates = [
            (
                min_x < -tolerance
                    && max_x < width - tolerance
                    && min_y > tolerance
                    && max_y < depth - tolerance,
                0_usize,
                0.0,
                true,
                std::f64::consts::PI,
                [0.0, min_y, max_x, max_y],
            ),
            (
                max_x > width + tolerance
                    && min_x > tolerance
                    && min_y > tolerance
                    && max_y < depth - tolerance,
                0,
                width,
                false,
                0.0,
                [min_x, min_y, width, max_y],
            ),
            (
                min_y < -tolerance
                    && max_y < depth - tolerance
                    && min_x > tolerance
                    && max_x < width - tolerance,
                1,
                0.0,
                true,
                3.0 * std::f64::consts::FRAC_PI_2,
                [min_x, 0.0, max_x, max_y],
            ),
            (
                max_y > depth + tolerance
                    && min_y > tolerance
                    && min_x > tolerance
                    && max_x < width - tolerance,
                1,
                depth,
                false,
                std::f64::consts::FRAC_PI_2,
                [min_x, min_y, max_x, depth],
            ),
        ];
        let mut candidates = candidates.into_iter().filter(|candidate| candidate.0);
        let (_, axis, limit, keep_greater, extreme_angle, bounds) = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }

        let ExactProfileSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } = self
            .segments
            .iter()
            .find(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))?
        else {
            unreachable!("filtered circular arc")
        };
        let start = start_bits.map(f64::from_bits);
        let end = end_bits.map(f64::from_bits);
        let center = center_bits.map(f64::from_bits);
        let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
        if !point_inside(center) || !radius.is_finite() || radius <= tolerance {
            return None;
        }
        let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
        if (end_radius - radius).abs() > tolerance {
            return None;
        }
        let distance = if keep_greater {
            center[axis] - limit
        } else {
            limit - center[axis]
        };
        if distance <= tolerance || distance >= radius - tolerance {
            return None;
        }
        let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
        let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
        let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)?;
        let intersection_offset = (distance / radius).acos();
        if !angle_on_directed_arc(start_angle, sweep, extreme_angle)
            || !angle_on_directed_arc(start_angle, sweep, extreme_angle - intersection_offset)
            || !angle_on_directed_arc(start_angle, sweep, extreme_angle + intersection_offset)
        {
            return None;
        }
        let outside_area = radius * radius * intersection_offset
            - distance * (radius * radius - distance * distance).sqrt();
        let overlap_area = f64::from_bits(self.area_bits) - outside_area;
        (outside_area > tolerance && overlap_area > tolerance).then_some((overlap_area, bounds))
    }

    #[must_use]
    pub fn strict_convex_line_arc_clipped_side_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        if !self.is_strict_convex_line_arc_profile()
            || self.segments.len() != 5
            || self
                .segments
                .iter()
                .filter(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
                .count()
                != 1
            || !width.is_finite()
            || !depth.is_finite()
            || width <= 0.0
            || depth <= 0.0
        {
            return None;
        }
        let tolerance = 1.0e-6;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let candidates = [
            (
                min_x < -tolerance
                    && max_x > tolerance
                    && max_x < width - tolerance
                    && min_y > tolerance
                    && max_y < depth - tolerance,
                0_usize,
                0.0,
                true,
                [0.0, min_y, max_x, max_y],
            ),
            (
                max_x > width + tolerance
                    && min_x > tolerance
                    && min_x < width - tolerance
                    && min_y > tolerance
                    && max_y < depth - tolerance,
                0,
                width,
                false,
                [min_x, min_y, width, max_y],
            ),
            (
                min_y < -tolerance
                    && max_y > tolerance
                    && max_y < depth - tolerance
                    && min_x > tolerance
                    && max_x < width - tolerance,
                1,
                0.0,
                true,
                [min_x, 0.0, max_x, max_y],
            ),
            (
                max_y > depth + tolerance
                    && min_y > tolerance
                    && min_y < depth - tolerance
                    && min_x > tolerance
                    && max_x < width - tolerance,
                1,
                depth,
                false,
                [min_x, min_y, max_x, depth],
            ),
        ];
        let mut candidates = candidates.into_iter().filter(|candidate| candidate.0);
        let (_, axis, limit, keep_greater, bounds) = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        let inside = |point: [f64; 2]| {
            if keep_greater {
                point[axis] > limit + tolerance
            } else {
                point[axis] < limit - tolerance
            }
        };
        let same_point = |left: [f64; 2], right: [f64; 2]| {
            left.into_iter()
                .zip(right)
                .all(|(left, right)| (left - right).abs() <= tolerance)
        };

        let ExactProfileSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } = self
            .segments
            .iter()
            .find(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))?
        else {
            unreachable!("filtered circular arc")
        };
        let arc_start = start_bits.map(f64::from_bits);
        let arc_end = end_bits.map(f64::from_bits);
        let center = center_bits.map(f64::from_bits);
        let start_inside = inside(arc_start);
        let end_inside = inside(arc_end);
        if start_inside == end_inside || !inside(center) {
            return None;
        }
        let radius = (arc_start[0] - center[0]).hypot(arc_start[1] - center[1]);
        let end_radius = (arc_end[0] - center[0]).hypot(arc_end[1] - center[1]);
        if !radius.is_finite() || radius <= tolerance || (end_radius - radius).abs() > tolerance {
            return None;
        }
        let start_angle = (arc_start[1] - center[1]).atan2(arc_start[0] - center[0]);
        let end_angle = (arc_end[1] - center[1]).atan2(arc_end[0] - center[0]);
        let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)?;
        let normalized_limit = (limit - center[axis]) / radius;
        if normalized_limit.abs() >= 1.0 - tolerance {
            return None;
        }
        let principal = if axis == 0 {
            normalized_limit.acos()
        } else {
            normalized_limit.asin()
        };
        let intersection_angles = if axis == 0 {
            [principal, -principal]
        } else {
            [principal, std::f64::consts::PI - principal]
        };
        let mut arc_intersections = intersection_angles.into_iter().filter_map(|angle| {
            if !angle_on_directed_arc(start_angle, sweep, angle) {
                return None;
            }
            let mut point = [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ];
            point[axis] = limit;
            (!same_point(point, arc_start) && !same_point(point, arc_end)).then_some((angle, point))
        });
        let (intersection_angle, arc_intersection) = arc_intersections.next()?;
        if arc_intersections.next().is_some() {
            return None;
        }

        let mut crossing_lines = self.segments.iter().filter_map(|segment| {
            let ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } = segment
            else {
                return None;
            };
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            (inside(start) != inside(end)).then_some((start, end))
        });
        let (line_start, line_end) = crossing_lines.next()?;
        if crossing_lines.next().is_some()
            || self.segments.iter().any(|segment| {
                let ExactProfileSegment::Line {
                    start_bits,
                    end_bits,
                } = segment
                else {
                    return false;
                };
                !inside(start_bits.map(f64::from_bits)) && !inside(end_bits.map(f64::from_bits))
            })
        {
            return None;
        }
        let outside_endpoint = if start_inside { arc_end } else { arc_start };
        if (start_inside && !same_point(line_start, outside_endpoint))
            || (!start_inside && !same_point(line_end, outside_endpoint))
        {
            return None;
        }
        let denominator = line_end[axis] - line_start[axis];
        if denominator.abs() <= tolerance {
            return None;
        }
        let t = (limit - line_start[axis]) / denominator;
        if t <= tolerance || t >= 1.0 - tolerance {
            return None;
        }
        let mut line_intersection = [
            line_start[0] + t * (line_end[0] - line_start[0]),
            line_start[1] + t * (line_end[1] - line_start[1]),
        ];
        line_intersection[axis] = limit;

        let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
        let arc_integral = |from: f64, to: f64| {
            let delta = directed_arc_sweep(from, to, *clockwise)?;
            Some(
                radius * center[0] * (to.sin() - from.sin())
                    - radius * center[1] * (to.cos() - from.cos())
                    + radius * radius * delta,
            )
        };
        let outside_twice_area = if start_inside {
            arc_integral(intersection_angle, end_angle)?
                + cross(outside_endpoint, line_intersection)
                + cross(line_intersection, arc_intersection)
        } else {
            cross(line_intersection, outside_endpoint)
                + arc_integral(start_angle, intersection_angle)?
                + cross(arc_intersection, line_intersection)
        };
        let outside_area = 0.5 * outside_twice_area.abs();
        let overlap_area = f64::from_bits(self.area_bits) - outside_area;
        (outside_area > tolerance && overlap_area > tolerance).then_some((overlap_area, bounds))
    }

    #[must_use]
    pub fn strict_convex_line_arc_clipped_south_east_corner_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        if !self.is_strict_convex_line_arc_profile()
            || self.segments.len() != 5
            || self
                .segments
                .iter()
                .filter(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
                .count()
                != 1
            || !width.is_finite()
            || !depth.is_finite()
            || width <= 0.0
            || depth <= 0.0
        {
            return None;
        }
        let tolerance = 1.0e-6;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        if min_x <= tolerance
            || min_x >= width - tolerance
            || max_x <= width + tolerance
            || min_y >= -tolerance
            || max_y <= tolerance
            || max_y >= depth - tolerance
        {
            return None;
        }
        let point_inside = |point: [f64; 2]| {
            point[0] > tolerance
                && point[0] < width - tolerance
                && point[1] > tolerance
                && point[1] < depth - tolerance
        };
        let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
        let ExactProfileSegment::Line {
            start_bits: first_line_start,
            end_bits: first_line_end,
        } = self
            .segments
            .iter()
            .find(|segment| matches!(segment, ExactProfileSegment::Line { .. }))?
        else {
            unreachable!("filtered first line")
        };
        if !point_inside(first_line_start.map(f64::from_bits))
            || !point_inside(first_line_end.map(f64::from_bits))
        {
            return None;
        }

        let ExactProfileSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } = self
            .segments
            .iter()
            .find(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))?
        else {
            unreachable!("filtered circular arc")
        };
        let arc_start = start_bits.map(f64::from_bits);
        let arc_end = end_bits.map(f64::from_bits);
        let center = center_bits.map(f64::from_bits);
        let start_inside = point_inside(arc_start);
        let end_inside = point_inside(arc_end);
        if start_inside == end_inside || !point_inside(center) {
            return None;
        }
        let outside_endpoint = if start_inside { arc_end } else { arc_start };
        if outside_endpoint[0] <= tolerance
            || outside_endpoint[0] >= width - tolerance
            || outside_endpoint[1] >= -tolerance
        {
            return None;
        }
        let radius = (arc_start[0] - center[0]).hypot(arc_start[1] - center[1]);
        let end_radius = (arc_end[0] - center[0]).hypot(arc_end[1] - center[1]);
        if !radius.is_finite() || radius <= tolerance || (end_radius - radius).abs() > tolerance {
            return None;
        }
        let start_angle = (arc_start[1] - center[1]).atan2(arc_start[0] - center[0]);
        let end_angle = (arc_end[1] - center[1]).atan2(arc_end[0] - center[0]);
        let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)?;
        let normalized_x = (width - center[0]) / radius;
        if normalized_x.abs() >= 1.0 - tolerance {
            return None;
        }
        let principal = normalized_x.acos();
        let mut east_contacts = [principal, -principal].into_iter().filter_map(|angle| {
            if !angle_on_directed_arc(start_angle, sweep, angle) {
                return None;
            }
            let point = [width, center[1] + radius * angle.sin()];
            (point[1] > tolerance && point[1] < depth - tolerance).then_some((angle, point))
        });
        let (east_angle, east_contact) = east_contacts.next()?;
        if east_contacts.next().is_some() {
            return None;
        }
        let normalized_y = -center[1] / radius;
        if normalized_y.abs() < 1.0 - tolerance {
            let principal = normalized_y.asin();
            if [principal, std::f64::consts::PI - principal]
                .into_iter()
                .filter(|angle| angle_on_directed_arc(start_angle, sweep, *angle))
                .any(|angle| {
                    let x = center[0] + radius * angle.cos();
                    x > tolerance && x < width - tolerance
                })
            {
                return None;
            }
        }
        let (inside_arc_start, inside_arc_end) = if start_inside {
            (start_angle, east_angle)
        } else {
            (east_angle, end_angle)
        };
        let inside_sweep = directed_arc_sweep(inside_arc_start, inside_arc_end, *clockwise)?;
        for angle in [
            0.0,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2,
        ] {
            if angle_on_directed_arc(inside_arc_start, inside_sweep, angle) {
                let point = [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ];
                if point[0] < -tolerance
                    || point[0] > width + tolerance
                    || point[1] < -tolerance
                    || point[1] > depth + tolerance
                {
                    return None;
                }
            }
        }

        let mut line_twice_area = 0.0;
        let mut inside_line_count = 0_u8;
        let mut outside_line_count = 0_u8;
        let mut overlap_min_x = arc_end[0].min(east_contact[0]);
        let mut south_contact = None::<([f64; 2], bool)>;
        for segment in &self.segments {
            let ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } = segment
            else {
                continue;
            };
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            if start[0] <= tolerance
                || start[0] >= width - tolerance
                || end[0] <= tolerance
                || end[0] >= width - tolerance
                || start[1] >= depth - tolerance
                || end[1] >= depth - tolerance
            {
                return None;
            }
            let start_inside = point_inside(start);
            let end_inside = point_inside(end);
            match (start_inside, end_inside) {
                (true, true) => {
                    inside_line_count += 1;
                    overlap_min_x = overlap_min_x.min(start[0]).min(end[0]);
                    line_twice_area += cross(start, end);
                }
                (false, false) => {
                    outside_line_count += 1;
                    if start[1] >= -tolerance || end[1] >= -tolerance {
                        return None;
                    }
                }
                _ => {
                    if south_contact.is_some() {
                        return None;
                    }
                    let denominator = end[1] - start[1];
                    if denominator.abs() <= tolerance {
                        return None;
                    }
                    let t = -start[1] / denominator;
                    if t <= tolerance || t >= 1.0 - tolerance {
                        return None;
                    }
                    let contact = [start[0] + t * (end[0] - start[0]), 0.0];
                    if contact[0] <= tolerance || contact[0] >= width - tolerance {
                        return None;
                    }
                    let inside_to_outside = start_inside;
                    overlap_min_x = overlap_min_x.min(contact[0]).min(if inside_to_outside {
                        start[0]
                    } else {
                        end[0]
                    });
                    line_twice_area += if inside_to_outside {
                        cross(start, contact)
                    } else {
                        cross(contact, end)
                    };
                    south_contact = Some((contact, inside_to_outside));
                }
            }
        }
        let (south_contact, line_inside_to_outside) = south_contact?;
        if inside_line_count != 2
            || outside_line_count != 1
            || start_inside
            || !line_inside_to_outside
        {
            return None;
        }
        let arc_integral = |from: f64, to: f64| {
            let delta = directed_arc_sweep(from, to, *clockwise)?;
            Some(
                radius * center[0] * (to.sin() - from.sin())
                    - radius * center[1] * (to.cos() - from.cos())
                    + radius * radius * delta,
            )
        };
        let arc_twice_area = arc_integral(inside_arc_start, inside_arc_end)?;
        let corner = [width, 0.0];
        let closure_twice_area = cross(south_contact, corner) + cross(corner, east_contact);
        let overlap_area = 0.5 * (line_twice_area + arc_twice_area + closure_twice_area).abs();
        (overlap_area > tolerance && overlap_area < width * depth - tolerance)
            .then_some((overlap_area, [overlap_min_x, 0.0, width, max_y.min(depth)]))
    }

    fn mirrored_across_vertical_axis(&self, width: f64) -> Option<Self> {
        if !width.is_finite() || width <= 0.0 {
            return None;
        }
        let mirror = |point_bits: [u64; 2]| {
            let [x, y] = point_bits.map(f64::from_bits);
            [(width - x).to_bits(), y.to_bits()]
        };
        Some(Self {
            segments: self
                .segments
                .iter()
                .map(|segment| match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => ExactProfileSegment::Line {
                        start_bits: mirror(*start_bits),
                        end_bits: mirror(*end_bits),
                    },
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => ExactProfileSegment::CircularArc {
                        start_bits: mirror(*start_bits),
                        end_bits: mirror(*end_bits),
                        center_bits: mirror(*center_bits),
                        clockwise: !*clockwise,
                    },
                })
                .collect(),
            bounds_bits: {
                let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
                [width - max_x, min_y, width - min_x, max_y].map(f64::to_bits)
            },
            area_bits: self.area_bits,
        })
    }

    fn mirrored_across_horizontal_axis(&self, depth: f64) -> Option<Self> {
        if !depth.is_finite() || depth <= 0.0 {
            return None;
        }
        let mirror = |point_bits: [u64; 2]| {
            let [x, y] = point_bits.map(f64::from_bits);
            [x.to_bits(), (depth - y).to_bits()]
        };
        Some(Self {
            segments: self
                .segments
                .iter()
                .map(|segment| match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => ExactProfileSegment::Line {
                        start_bits: mirror(*start_bits),
                        end_bits: mirror(*end_bits),
                    },
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => ExactProfileSegment::CircularArc {
                        start_bits: mirror(*start_bits),
                        end_bits: mirror(*end_bits),
                        center_bits: mirror(*center_bits),
                        clockwise: !*clockwise,
                    },
                })
                .collect(),
            bounds_bits: {
                let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
                [min_x, depth - max_y, max_x, depth - min_y].map(f64::to_bits)
            },
            area_bits: self.area_bits,
        })
    }

    #[must_use]
    pub fn strict_convex_line_arc_clipped_north_east_corner_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        self.mirrored_across_horizontal_axis(depth)?
            .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
            .map(|(area, [min_x, min_y, max_x, max_y])| {
                (area, [min_x, depth - max_y, max_x, depth - min_y])
            })
    }

    #[must_use]
    pub fn strict_convex_line_arc_clipped_north_west_corner_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        self.mirrored_across_vertical_axis(width)?
            .strict_convex_line_arc_clipped_north_east_corner_overlap(width, depth)
            .map(|(area, [min_x, min_y, max_x, max_y])| {
                (area, [width - max_x, min_y, width - min_x, max_y])
            })
    }

    #[must_use]
    pub fn strict_convex_line_arc_clipped_south_west_corner_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        self.mirrored_across_vertical_axis(width)?
            .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
            .map(|(area, [min_x, min_y, max_x, max_y])| {
                (area, [width - max_x, min_y, width - min_x, max_y])
            })
    }

    #[must_use]
    pub fn is_line_arc_d_profile(&self) -> bool {
        self.segments.len() == 2
            && self
                .segments
                .iter()
                .filter(|segment| matches!(segment, ExactProfileSegment::Line { .. }))
                .count()
                == 1
            && self
                .segments
                .iter()
                .filter(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
                .count()
                == 1
    }

    #[must_use]
    pub fn d_profile_arc_only_clipped_side_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        if !self.is_line_arc_d_profile()
            || !width.is_finite()
            || width <= 0.0
            || !depth.is_finite()
            || depth <= 0.0
        {
            return None;
        }
        let tolerance = 1.0e-6;
        let point_inside = |point: [f64; 2]| {
            point[0] > tolerance
                && point[0] < width - tolerance
                && point[1] > tolerance
                && point[1] < depth - tolerance
        };
        let line = self.segments.iter().find_map(|segment| match segment {
            ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } => Some((start_bits.map(f64::from_bits), end_bits.map(f64::from_bits))),
            ExactProfileSegment::CircularArc { .. } => None,
        })?;
        let ExactProfileSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } = self
            .segments
            .iter()
            .find(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))?
        else {
            unreachable!("filtered circular arc")
        };
        let start = start_bits.map(f64::from_bits);
        let end = end_bits.map(f64::from_bits);
        let center = center_bits.map(f64::from_bits);
        let same_point = |left: [f64; 2], right: [f64; 2]| {
            (left[0] - right[0]).abs() <= tolerance && (left[1] - right[1]).abs() <= tolerance
        };
        if !((same_point(line.0, start) && same_point(line.1, end))
            || (same_point(line.0, end) && same_point(line.1, start)))
            || !point_inside(start)
            || !point_inside(end)
            || !point_inside(center)
        {
            return None;
        }
        let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
        let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
        if !radius.is_finite()
            || radius <= tolerance
            || (end_radius - radius).abs() > tolerance
            || (center[0] * 2.0 - start[0] - end[0]).abs() > tolerance
            || (center[1] * 2.0 - start[1] - end[1]).abs() > tolerance
        {
            return None;
        }
        let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
        let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
        let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)?;
        if (sweep.abs() - std::f64::consts::PI).abs() > tolerance {
            return None;
        }

        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let candidates = [
            (
                min_x < -tolerance && max_x < width - tolerance,
                0_usize,
                0.0,
                true,
                std::f64::consts::PI,
                [0.0, min_y, max_x, max_y],
            ),
            (
                max_x > width + tolerance && min_x > tolerance,
                0,
                width,
                false,
                0.0,
                [min_x, min_y, width, max_y],
            ),
            (
                min_y < -tolerance && max_y < depth - tolerance,
                1,
                0.0,
                true,
                3.0 * std::f64::consts::FRAC_PI_2,
                [min_x, 0.0, max_x, max_y],
            ),
            (
                max_y > depth + tolerance && min_y > tolerance,
                1,
                depth,
                false,
                std::f64::consts::FRAC_PI_2,
                [min_x, min_y, max_x, depth],
            ),
        ];
        let mut candidates = candidates.into_iter().filter(|candidate| candidate.0);
        let (_, axis, limit, keep_greater, extreme_angle, bounds) = candidates.next()?;
        if candidates.next().is_some() {
            return None;
        }
        let distance = if keep_greater {
            center[axis] - limit
        } else {
            limit - center[axis]
        };
        if distance <= tolerance || distance >= radius - tolerance {
            return None;
        }
        let intersection_offset = (distance / radius).acos();
        if !angle_on_directed_arc(start_angle, sweep, extreme_angle)
            || !angle_on_directed_arc(start_angle, sweep, extreme_angle - intersection_offset)
            || !angle_on_directed_arc(start_angle, sweep, extreme_angle + intersection_offset)
        {
            return None;
        }
        let outside_area = radius * radius * intersection_offset
            - distance * (radius * radius - distance * distance).sqrt();
        let overlap_area = f64::from_bits(self.area_bits) - outside_area;
        (outside_area > tolerance && overlap_area > tolerance).then_some((overlap_area, bounds))
    }

    #[must_use]
    pub fn is_line_arc_capsule_profile(&self) -> bool {
        let vectors = |arc: bool| {
            self.segments
                .iter()
                .filter_map(|segment| match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } if !arc => Some((
                        start_bits.map(f64::from_bits),
                        end_bits.map(f64::from_bits),
                        None,
                    )),
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        ..
                    } if arc => Some((
                        start_bits.map(f64::from_bits),
                        end_bits.map(f64::from_bits),
                        Some(center_bits.map(f64::from_bits)),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let lines = vectors(false);
        let arcs = vectors(true);
        if self.segments.len() != 4 || lines.len() != 2 || arcs.len() != 2 {
            return false;
        }
        let vector = |(start, end, _): &([f64; 2], [f64; 2], Option<[f64; 2]>)| {
            [end[0] - start[0], end[1] - start[1]]
        };
        let line = vector(&lines[0]);
        let opposite_line = vector(&lines[1]);
        let diameter = vector(&arcs[0]);
        let opposite_diameter = vector(&arcs[1]);
        let scale = line
            .into_iter()
            .chain(opposite_line)
            .chain(diameter)
            .chain(opposite_diameter)
            .map(f64::abs)
            .fold(1.0, f64::max);
        let tolerance = scale * 1.0e-9;
        let opposite = |left: [f64; 2], right: [f64; 2]| {
            (left[0] + right[0]).abs() <= tolerance && (left[1] + right[1]).abs() <= tolerance
        };
        opposite(line, opposite_line)
            && opposite(diameter, opposite_diameter)
            && (line[0] * diameter[0] + line[1] * diameter[1]).abs() <= tolerance * scale
            && arcs.iter().all(|(start, end, center)| {
                let center = center.expect("filtered circular arc");
                (center[0] * 2.0 - start[0] - end[0]).abs() <= tolerance
                    && (center[1] * 2.0 - start[1] - end[1]).abs() <= tolerance
            })
    }

    #[must_use]
    pub fn capsule_side_overlap(&self, width: f64, depth: f64) -> Option<(f64, [f64; 4])> {
        if !self.is_line_arc_capsule_profile()
            || !width.is_finite()
            || width <= 0.0
            || !depth.is_finite()
            || depth <= 0.0
        {
            return None;
        }
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let scale = [width, depth, min_x, min_y, max_x, max_y]
            .into_iter()
            .map(f64::abs)
            .fold(1.0, f64::max);
        let tolerance = scale * 1.0e-9;
        let mut arcs = self
            .segments
            .iter()
            .filter_map(|segment| match segment {
                ExactProfileSegment::CircularArc {
                    start_bits,
                    end_bits,
                    center_bits,
                    ..
                } => {
                    let start = start_bits.map(f64::from_bits);
                    let end = end_bits.map(f64::from_bits);
                    let center = center_bits.map(f64::from_bits);
                    Some((
                        center,
                        (start[0] - center[0]).hypot(start[1] - center[1]),
                        [end[0] - start[0], end[1] - start[1]],
                    ))
                }
                ExactProfileSegment::Line { .. } => None,
            })
            .collect::<Vec<_>>();
        let horizontal = arcs.iter().all(|(_, radius, diameter)| {
            diameter[0].abs() <= tolerance && (diameter[1].abs() - 2.0 * radius).abs() <= tolerance
        });
        let vertical = arcs.iter().all(|(_, radius, diameter)| {
            diameter[1].abs() <= tolerance && (diameter[0].abs() - 2.0 * radius).abs() <= tolerance
        });
        if horizontal == vertical {
            return None;
        }
        let axis = usize::from(vertical);
        let cross_axis = 1 - axis;
        let extent = [width, depth];
        let profile_min = [min_x, min_y];
        let profile_max = [max_x, max_y];
        arcs.sort_by(|left, right| left.0[axis].total_cmp(&right.0[axis]));
        let [(low_center, low_radius, _), (high_center, high_radius, _)] = arcs.as_slice() else {
            return None;
        };
        if (low_center[cross_axis] - high_center[cross_axis]).abs() > tolerance
            || (low_radius - high_radius).abs() > tolerance
            || *low_radius <= tolerance
            || high_center[axis] - low_center[axis] <= tolerance
            || profile_min[cross_axis] <= tolerance
            || profile_max[cross_axis] >= extent[cross_axis] - tolerance
        {
            return None;
        }
        let diameter = 2.0 * low_radius;
        let semicircle_area = 0.5 * std::f64::consts::PI * low_radius * low_radius;
        if profile_min[axis] > tolerance
            && low_center[axis] < extent[axis] - tolerance
            && high_center[axis] > extent[axis] + tolerance
        {
            let mut bounds = [min_x, min_y, max_x, max_y];
            bounds[axis + 2] = extent[axis];
            Some((
                semicircle_area + (extent[axis] - low_center[axis]) * diameter,
                bounds,
            ))
        } else if profile_max[axis] < extent[axis] - tolerance
            && low_center[axis] < -tolerance
            && high_center[axis] > tolerance
        {
            let mut bounds = [min_x, min_y, max_x, max_y];
            bounds[axis] = 0.0;
            Some((semicircle_area + high_center[axis] * diameter, bounds))
        } else {
            None
        }
    }

    #[must_use]
    pub fn capsule_corner_overlap(&self, width: f64, depth: f64) -> Option<(f64, [f64; 4])> {
        if !self.is_line_arc_capsule_profile()
            || !width.is_finite()
            || width <= 0.0
            || !depth.is_finite()
            || depth <= 0.0
        {
            return None;
        }
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let scale = [width, depth, min_x, min_y, max_x, max_y]
            .into_iter()
            .map(f64::abs)
            .fold(1.0, f64::max);
        let tolerance = scale * 1.0e-9;
        let mut arcs = self
            .segments
            .iter()
            .filter_map(|segment| match segment {
                ExactProfileSegment::CircularArc {
                    start_bits,
                    end_bits,
                    center_bits,
                    ..
                } => {
                    let start = start_bits.map(f64::from_bits);
                    let end = end_bits.map(f64::from_bits);
                    let center = center_bits.map(f64::from_bits);
                    Some((
                        center,
                        (start[0] - center[0]).hypot(start[1] - center[1]),
                        [end[0] - start[0], end[1] - start[1]],
                    ))
                }
                ExactProfileSegment::Line { .. } => None,
            })
            .collect::<Vec<_>>();
        let horizontal = arcs.iter().all(|(_, radius, diameter)| {
            diameter[0].abs() <= tolerance && (diameter[1].abs() - 2.0 * radius).abs() <= tolerance
        });
        let vertical = arcs.iter().all(|(_, radius, diameter)| {
            diameter[1].abs() <= tolerance && (diameter[0].abs() - 2.0 * radius).abs() <= tolerance
        });
        if horizontal == vertical {
            return None;
        }
        let axis = usize::from(vertical);
        let cross_axis = 1 - axis;
        let extent = [width, depth];
        let profile_min = [min_x, min_y];
        let profile_max = [max_x, max_y];
        arcs.sort_by(|left, right| left.0[axis].total_cmp(&right.0[axis]));
        let [(low_center, low_radius, _), (high_center, high_radius, _)] = arcs.as_slice() else {
            return None;
        };
        if (low_center[cross_axis] - high_center[cross_axis]).abs() > tolerance
            || (low_radius - high_radius).abs() > tolerance
            || *low_radius <= tolerance
            || high_center[axis] - low_center[axis] <= tolerance
        {
            return None;
        }
        let radius = *low_radius;
        let axis_length = if profile_min[axis] > tolerance
            && low_center[axis] < extent[axis] - tolerance
            && high_center[axis] > extent[axis] + tolerance
        {
            extent[axis] - low_center[axis]
        } else if profile_max[axis] < extent[axis] - tolerance
            && low_center[axis] < -tolerance
            && high_center[axis] > tolerance
        {
            high_center[axis]
        } else {
            return None;
        };
        let cross_center = low_center[cross_axis];
        let cross_distance = if profile_min[cross_axis] < -tolerance
            && cross_center > tolerance
            && profile_max[cross_axis] < extent[cross_axis] - tolerance
        {
            cross_center
        } else if profile_max[cross_axis] > extent[cross_axis] + tolerance
            && cross_center < extent[cross_axis] - tolerance
            && profile_min[cross_axis] > tolerance
        {
            extent[cross_axis] - cross_center
        } else {
            return None;
        };
        if cross_distance >= radius - tolerance {
            return None;
        }
        let chord_half = (radius * radius - cross_distance * cross_distance).sqrt();
        let clipped_segment =
            radius * radius * (cross_distance / radius).acos() - cross_distance * chord_half;
        let retained_cap_area = 0.5 * (std::f64::consts::PI * radius * radius - clipped_segment);
        let retained_height = radius + cross_distance;
        Some((
            axis_length * retained_height + retained_cap_area,
            [
                min_x.max(0.0),
                min_y.max(0.0),
                max_x.min(width),
                max_y.min(depth),
            ],
        ))
    }

    #[must_use]
    pub fn is_line_arc_rounded_rectangle_profile(&self) -> bool {
        if self.segments.len() != 8
            || self.segments.iter().enumerate().any(|(index, segment)| {
                matches!(segment, ExactProfileSegment::Line { .. }) != index.is_multiple_of(2)
            })
        {
            return false;
        }
        let scale = self
            .bounds_bits
            .map(f64::from_bits)
            .into_iter()
            .map(f64::abs)
            .fold(1.0, f64::max);
        let tolerance = scale * 1.0e-9;
        let lines = self
            .segments
            .iter()
            .step_by(2)
            .map(|segment| match segment {
                ExactProfileSegment::Line {
                    start_bits,
                    end_bits,
                } => {
                    let start = start_bits.map(f64::from_bits);
                    let end = end_bits.map(f64::from_bits);
                    [end[0] - start[0], end[1] - start[1]]
                }
                ExactProfileSegment::CircularArc { .. } => unreachable!(),
            })
            .collect::<Vec<_>>();
        let opposite = |left: [f64; 2], right: [f64; 2]| {
            (left[0] + right[0]).abs() <= tolerance && (left[1] + right[1]).abs() <= tolerance
        };
        if lines
            .iter()
            .any(|line| (line[0].abs() <= tolerance) == (line[1].abs() <= tolerance))
            || !opposite(lines[0], lines[2])
            || !opposite(lines[1], lines[3])
            || (lines[0][0] * lines[1][0] + lines[0][1] * lines[1][1]).abs() > tolerance * scale
        {
            return false;
        }
        let mut radius = None::<f64>;
        let mut clockwise = None::<bool>;
        self.segments
            .iter()
            .enumerate()
            .skip(1)
            .step_by(2)
            .all(|(index, segment)| {
                let ExactProfileSegment::CircularArc {
                    start_bits,
                    end_bits,
                    center_bits,
                    clockwise: arc_clockwise,
                } = segment
                else {
                    return false;
                };
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let start_radius = [start[0] - center[0], start[1] - center[1]];
                let end_radius = [end[0] - center[0], end[1] - center[1]];
                let arc_radius = start_radius[0].hypot(start_radius[1]);
                let cross = start_radius[0] * end_radius[1] - start_radius[1] * end_radius[0];
                let previous_line = lines[(index - 1) / 2];
                let next_line = lines[index.div_ceil(2) % lines.len()];
                let valid = arc_radius > tolerance
                    && (arc_radius - end_radius[0].hypot(end_radius[1])).abs() <= tolerance
                    && (start_radius[0] * end_radius[0] + start_radius[1] * end_radius[1]).abs()
                        <= tolerance * scale
                    && (previous_line[0] * start_radius[0] + previous_line[1] * start_radius[1])
                        .abs()
                        <= tolerance * scale
                    && (next_line[0] * end_radius[0] + next_line[1] * end_radius[1]).abs()
                        <= tolerance * scale
                    && if *arc_clockwise {
                        cross < -tolerance
                    } else {
                        cross > tolerance
                    }
                    && radius.is_none_or(|expected| (arc_radius - expected).abs() <= tolerance)
                    && clockwise.is_none_or(|expected| expected == *arc_clockwise);
                radius.get_or_insert(arc_radius);
                clockwise.get_or_insert(*arc_clockwise);
                valid
            })
    }

    #[must_use]
    pub fn rounded_rectangle_side_overlap_area(&self, width: f64, depth: f64) -> Option<f64> {
        if !self.is_line_arc_rounded_rectangle_profile() {
            return None;
        }
        let radius = self.segments.iter().find_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((start[0] - center[0]).hypot(start[1] - center[1]))
            }
            ExactProfileSegment::Line { .. } => None,
        })?;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let tolerance = 1.0e-6;
        let covers_width = min_x + radius <= tolerance && max_x - radius >= width - tolerance;
        let covers_depth = min_y + radius <= tolerance && max_y - radius >= depth - tolerance;
        if covers_depth
            && min_x > tolerance
            && min_x < width - tolerance
            && max_x > width + tolerance
        {
            Some((width - min_x) * depth)
        } else if covers_depth
            && min_x < -tolerance
            && max_x > tolerance
            && max_x < width - tolerance
        {
            Some(max_x * depth)
        } else if covers_width
            && min_y > tolerance
            && min_y < depth - tolerance
            && max_y > depth + tolerance
        {
            Some((depth - min_y) * width)
        } else if covers_width
            && min_y < -tolerance
            && max_y > tolerance
            && max_y < depth - tolerance
        {
            Some(max_y * width)
        } else {
            None
        }
    }

    #[must_use]
    pub fn rounded_rectangle_chord_side_overlap(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<(f64, [f64; 4])> {
        if !self.is_line_arc_rounded_rectangle_profile()
            || !width.is_finite()
            || width <= 0.0
            || !depth.is_finite()
            || depth <= 0.0
        {
            return None;
        }
        let radius = self.segments.iter().find_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((start[0] - center[0]).hypot(start[1] - center[1]))
            }
            ExactProfileSegment::Line { .. } => None,
        })?;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let scale = [width, depth, min_x, min_y, max_x, max_y, radius]
            .into_iter()
            .map(f64::abs)
            .fold(1.0, f64::max);
        let tolerance = scale * 1.0e-9;
        if !radius.is_finite()
            || radius <= tolerance
            || max_x - min_x <= 2.0 * radius + tolerance
            || max_y - min_y <= 2.0 * radius + tolerance
        {
            return None;
        }
        let corner_deficit = 2.0 * radius * radius * (1.0 - std::f64::consts::PI / 4.0);
        if min_y > tolerance
            && max_y < depth - tolerance
            && min_x > tolerance
            && min_x + radius < width - tolerance
            && max_x - radius > width + tolerance
        {
            Some((
                (width - min_x) * (max_y - min_y) - corner_deficit,
                [min_x, min_y, width, max_y],
            ))
        } else if min_y > tolerance
            && max_y < depth - tolerance
            && max_x < width - tolerance
            && min_x + radius < -tolerance
            && max_x - radius > tolerance
        {
            Some((
                max_x * (max_y - min_y) - corner_deficit,
                [0.0, min_y, max_x, max_y],
            ))
        } else if min_x > tolerance
            && max_x < width - tolerance
            && min_y > tolerance
            && min_y + radius < depth - tolerance
            && max_y - radius > depth + tolerance
        {
            Some((
                (depth - min_y) * (max_x - min_x) - corner_deficit,
                [min_x, min_y, max_x, depth],
            ))
        } else if min_x > tolerance
            && max_x < width - tolerance
            && max_y < depth - tolerance
            && min_y + radius < -tolerance
            && max_y - radius > tolerance
        {
            Some((
                max_y * (max_x - min_x) - corner_deficit,
                [min_x, 0.0, max_x, max_y],
            ))
        } else {
            None
        }
    }

    #[must_use]
    pub fn rounded_rectangle_corner_overlap_area(&self, width: f64, depth: f64) -> Option<f64> {
        if !self.is_line_arc_rounded_rectangle_profile() {
            return None;
        }
        let radius = self.segments.iter().find_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((start[0] - center[0]).hypot(start[1] - center[1]))
            }
            ExactProfileSegment::Line { .. } => None,
        })?;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let tolerance = 1.0e-6;
        let overlap_width = if min_x > tolerance
            && min_x + radius < width - tolerance
            && max_x - radius > width + tolerance
        {
            width - min_x
        } else if min_x + radius < -tolerance
            && max_x - radius > tolerance
            && max_x < width - tolerance
        {
            max_x
        } else {
            return None;
        };
        let overlap_depth = if min_y > tolerance
            && min_y + radius < depth - tolerance
            && max_y - radius > depth + tolerance
        {
            depth - min_y
        } else if min_y + radius < -tolerance
            && max_y - radius > tolerance
            && max_y < depth - tolerance
        {
            max_y
        } else {
            return None;
        };
        Some(overlap_width * overlap_depth - radius * radius * (1.0 - std::f64::consts::FRAC_PI_4))
    }

    #[must_use]
    pub fn rounded_rectangle_arc_clipped_corner_overlap_area(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<f64> {
        if !self.is_line_arc_rounded_rectangle_profile() {
            return None;
        }
        let radius = self.segments.iter().find_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((start[0] - center[0]).hypot(start[1] - center[1]))
            }
            ExactProfileSegment::Line { .. } => None,
        })?;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let tolerance = 1.0e-6;
        let classify_axis = |min: f64, max: f64, limit: f64| {
            let inward = if min > tolerance && min < limit - tolerance && max > limit + tolerance {
                limit - (min + radius)
            } else if min < -tolerance && max > tolerance && max < limit - tolerance {
                max - radius
            } else {
                return None;
            };
            if inward > tolerance {
                Some((false, inward))
            } else if inward < -tolerance && -inward < radius - tolerance {
                Some((true, -inward))
            } else {
                None
            }
        };
        let (x_clipped, x_distance) = classify_axis(min_x, max_x, width)?;
        let (y_clipped, y_distance) = classify_axis(min_y, max_y, depth)?;
        if x_clipped == y_clipped {
            return None;
        }
        let (clip_distance, straight_extension) = if x_clipped {
            (x_distance, y_distance)
        } else {
            (y_distance, x_distance)
        };
        let chord_half = (radius * radius - clip_distance * clip_distance).sqrt();
        let circular_segment = radius * radius * std::f64::consts::FRAC_PI_4
            - 0.5
                * (clip_distance * chord_half + radius * radius * (clip_distance / radius).asin());
        Some((radius - clip_distance) * straight_extension + circular_segment)
    }

    #[must_use]
    pub fn rounded_rectangle_arc_clipped_corner_overlap_bounds(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<[f64; 4]> {
        self.rounded_rectangle_arc_clipped_corner_overlap_area(width, depth)?;
        let radius = self.segments.iter().find_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((start[0] - center[0]).hypot(start[1] - center[1]))
            }
            ExactProfileSegment::Line { .. } => None,
        })?;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let tolerance = 1.0e-6;
        let classify_axis = |min: f64, max: f64, limit: f64| {
            if min > tolerance && min < limit - tolerance && max > limit + tolerance {
                let center = min + radius;
                Some((true, center, center > limit + tolerance))
            } else if min < -tolerance && max > tolerance && max < limit - tolerance {
                let center = max - radius;
                Some((false, center, center < -tolerance))
            } else {
                None
            }
        };
        let (x_upper, x_center, x_clipped) = classify_axis(min_x, max_x, width)?;
        let (y_upper, y_center, y_clipped) = classify_axis(min_y, max_y, depth)?;
        if x_clipped == y_clipped {
            return None;
        }
        let mut bounds = [
            min_x.max(0.0),
            min_y.max(0.0),
            max_x.min(width),
            max_y.min(depth),
        ];
        if x_clipped {
            let clip_distance = if x_upper { x_center - width } else { -x_center };
            let chord_half = (radius * radius - clip_distance * clip_distance).sqrt();
            if y_upper {
                bounds[1] = y_center - chord_half;
            } else {
                bounds[3] = y_center + chord_half;
            }
        } else {
            let clip_distance = if y_upper { y_center - depth } else { -y_center };
            let chord_half = (radius * radius - clip_distance * clip_distance).sqrt();
            if x_upper {
                bounds[0] = x_center - chord_half;
            } else {
                bounds[2] = x_center + chord_half;
            }
        }
        Some(bounds)
    }

    #[must_use]
    pub fn rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<f64> {
        if !self.is_line_arc_rounded_rectangle_profile() {
            return None;
        }
        let radius = self.segments.iter().find_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((start[0] - center[0]).hypot(start[1] - center[1]))
            }
            ExactProfileSegment::Line { .. } => None,
        })?;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let tolerance = 1.0e-6;
        let clipped_distance = |min: f64, max: f64, limit: f64| {
            let inward = if min > tolerance && min < limit - tolerance && max > limit + tolerance {
                limit - (min + radius)
            } else if min < -tolerance && max > tolerance && max < limit - tolerance {
                max - radius
            } else {
                return None;
            };
            (-inward > tolerance && -inward < radius - tolerance).then_some(-inward)
        };
        let x_distance = clipped_distance(min_x, max_x, width)?;
        let y_distance = clipped_distance(min_y, max_y, depth)?;
        if x_distance * x_distance + y_distance * y_distance >= radius * radius - tolerance {
            return None;
        }
        let x_limit = (radius * radius - y_distance * y_distance).sqrt();
        let primitive = |value: f64| {
            0.5 * (value * (radius * radius - value * value).sqrt()
                + radius * radius * (value / radius).asin())
        };
        Some(primitive(x_limit) - primitive(x_distance) - y_distance * (x_limit - x_distance))
    }

    #[must_use]
    pub fn rounded_rectangle_two_axis_arc_clipped_corner_overlap_bounds(
        &self,
        width: f64,
        depth: f64,
    ) -> Option<[f64; 4]> {
        self.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(width, depth)?;
        let radius = self.segments.iter().find_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((start[0] - center[0]).hypot(start[1] - center[1]))
            }
            ExactProfileSegment::Line { .. } => None,
        })?;
        let [min_x, min_y, max_x, max_y] = self.bounds_bits.map(f64::from_bits);
        let tolerance = 1.0e-6;
        let clipped_axis = |min: f64, max: f64, limit: f64| {
            if min > tolerance && min < limit - tolerance && max > limit + tolerance {
                let center = min + radius;
                Some((true, center, center - limit))
            } else if min < -tolerance && max > tolerance && max < limit - tolerance {
                let center = max - radius;
                Some((false, center, -center))
            } else {
                None
            }
        };
        let (x_upper, x_center, x_distance) = clipped_axis(min_x, max_x, width)?;
        let (y_upper, y_center, y_distance) = clipped_axis(min_y, max_y, depth)?;
        let x_chord_half = (radius * radius - y_distance * y_distance).sqrt();
        let y_chord_half = (radius * radius - x_distance * x_distance).sqrt();
        let mut bounds = [
            min_x.max(0.0),
            min_y.max(0.0),
            max_x.min(width),
            max_y.min(depth),
        ];
        if x_upper {
            bounds[0] = x_center - x_chord_half;
        } else {
            bounds[2] = x_center + x_chord_half;
        }
        if y_upper {
            bounds[1] = y_center - y_chord_half;
        } else {
            bounds[3] = y_center + y_chord_half;
        }
        Some(bounds)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactBooleanRequest {
    pub feature_id: FeatureId,
    pub operation: BooleanOperation,
    pub target_feature_id: FeatureId,
    pub tool_feature_id: FeatureId,
    pub profile_feature_id: FeatureId,
    pub min_x_bits: u64,
    pub min_y_bits: u64,
    pub width_bits: u64,
    pub depth_bits: u64,
    pub circle: Option<ExactCircleProfile>,
    pub profile: Option<ExactMixedProfile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactBoxShellRequest {
    pub shell_feature_id: FeatureId,
    pub thickness_bits: u64,
    pub edge_finish_feature_id: Option<FeatureId>,
    pub edge_finish_kind: Option<EdgeFinishKind>,
    pub edge_finish_amount_bits: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactPlanarOffsetRequest {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub offset_feature_id: FeatureId,
    pub source_bounds_bits: [u64; 4],
    pub distance_bits: u64,
    pub canonical_input_digest: String,
}

impl ExactPlanarOffsetRequest {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        let [profile_feature_id, offset_feature_id] = definition.feature_ids() else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        let profile = snapshot
            .feature(*profile_feature_id)
            .ok_or(ExactProductError::ProfileNotFound(*profile_feature_id))?;
        let offset = snapshot
            .feature(*offset_feature_id)
            .ok_or(ExactProductError::UnsupportedDefinition)?;
        let FeatureKind::Profile { points_mm } = profile.kind() else {
            return Err(ExactProductError::UnsupportedProfile);
        };
        let FeatureKind::PlanarOffset {
            profile: offset_profile_id,
            distance,
        } = offset.kind()
        else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        if offset_profile_id != profile_feature_id {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let source_bounds =
            rectangle_bounds(points_mm).ok_or(ExactProductError::UnsupportedProfile)?;
        let distance_mm = distance.millimetres();
        let output_bounds = [
            source_bounds[0] - distance_mm,
            source_bounds[1] - distance_mm,
            source_bounds[2] + distance_mm,
            source_bounds[3] + distance_mm,
        ];
        if !distance_mm.is_finite()
            || distance_mm.abs() < EXACT_MIN_LENGTH_MM
            || output_bounds.into_iter().any(|value| !value.is_finite())
            || output_bounds[2] - output_bounds[0] < EXACT_MIN_LENGTH_MM
            || output_bounds[3] - output_bounds[1] < EXACT_MIN_LENGTH_MM
        {
            return Err(ExactProductError::UnsupportedProfile);
        }
        let source_digest = snapshot.canonical_digest();
        let source_bounds_bits = source_bounds.map(f64::to_bits);
        let distance_bits = distance_mm.to_bits();
        let canonical_input_digest = digest(&format!(
            "{}:{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
            EXACT_PLANAR_OFFSET_SCHEMA_V1,
            snapshot.document_id().0,
            snapshot.revision_id(),
            definition_id.0,
            profile_feature_id.0,
            offset_feature_id.0,
            source_bounds_bits[0],
            source_bounds_bits[1],
            source_bounds_bits[2],
            source_bounds_bits[3],
            distance_bits,
            source_digest,
        ));
        Ok(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id: *profile_feature_id,
            offset_feature_id: *offset_feature_id,
            source_bounds_bits,
            distance_bits,
            canonical_input_digest,
        })
    }

    #[must_use]
    pub fn source_bounds_mm(&self) -> [f64; 4] {
        self.source_bounds_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn distance_mm(&self) -> f64 {
        f64::from_bits(self.distance_bits)
    }

    #[must_use]
    pub fn expected_bounds_mm(&self) -> [[f64; 3]; 2] {
        let [min_x, min_y, max_x, max_y] = self.source_bounds_mm();
        let distance = self.distance_mm();
        [
            [min_x - distance, min_y - distance, 0.0],
            [max_x + distance, max_y + distance, 0.0],
        ]
    }

    #[must_use]
    pub fn expected_area_mm2(&self) -> f64 {
        let [min, max] = self.expected_bounds_mm();
        (max[0] - min[0]) * (max[1] - min[1])
    }

    #[must_use]
    pub const fn producer_feature_id(&self) -> FeatureId {
        self.offset_feature_id
    }

    #[must_use]
    pub const fn evaluator(&self) -> &'static str {
        EXACT_PLANAR_OFFSET_EVALUATOR_V1
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactPlanarOffsetIdentity {
    pub schema: String,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub offset_feature_id: FeatureId,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactPlanarOffsetPackage {
    pub identity: ExactPlanarOffsetIdentity,
    pub bounds_mm: [[f64; 3]; 2],
    pub area_mm2: f64,
    pub vertices: Vec<ExactVertex>,
    pub triangles: Vec<ExactTriangle>,
    pub reference: BodySubshapeRef,
}

impl ExactPlanarOffsetPackage {
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.document_id == snapshot.document_id()
            && self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && ExactPlanarOffsetRequest::from_snapshot(snapshot, self.identity.definition_id)
                .is_ok_and(|request| self.validate_for_request(&request).is_ok())
    }

    pub fn validate_for_request(
        &self,
        request: &ExactPlanarOffsetRequest,
    ) -> Result<(), ExactProductError> {
        let expected_bounds = request.expected_bounds_mm();
        let expected_vertices = [
            [expected_bounds[0][0], expected_bounds[0][1], 0.0],
            [expected_bounds[1][0], expected_bounds[0][1], 0.0],
            [expected_bounds[1][0], expected_bounds[1][1], 0.0],
            [expected_bounds[0][0], expected_bounds[1][1], 0.0],
        ];
        if self.identity.schema != EXACT_PLANAR_OFFSET_SCHEMA_V1
            || self.identity.document_id != request.document_id
            || self.identity.source_revision != request.source_revision
            || self.identity.source_digest != request.source_digest
            || self.identity.definition_id != request.definition_id
            || self.identity.profile_feature_id != request.profile_feature_id
            || self.identity.offset_feature_id != request.offset_feature_id
            || self.identity.canonical_input_digest != request.canonical_input_digest
            || self.identity.evaluator != request.evaluator()
            || self.identity.exact_input_digest.is_empty()
            || self.identity.result_fingerprint.is_empty()
            || self.identity.backend.is_empty()
            || self.identity.tolerance.is_empty()
            || self.bounds_mm != expected_bounds
            || (self.area_mm2 - request.expected_area_mm2()).abs() > 1.0e-6
            || self.vertices.len() != 4
            || self
                .vertices
                .iter()
                .zip(expected_vertices)
                .any(|(actual, expected)| actual.position_mm != expected)
            || self.triangles
                != [
                    ExactTriangle {
                        vertex_indices: [0, 1, 2],
                        face_role: Some(ExactFaceRole::PlanarOffsetFace),
                    },
                    ExactTriangle {
                        vertex_indices: [0, 2, 3],
                        face_role: Some(ExactFaceRole::PlanarOffsetFace),
                    },
                ]
            || !self.reference.matches_planar_offset_request(request)
            || self.reference.exact_input_digest != self.identity.exact_input_digest
            || self.reference.result_fingerprint != self.identity.result_fingerprint
            || self.reference.backend != self.identity.backend
            || self.reference.tolerance != self.identity.tolerance
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactSweepRequest {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub path_feature_id: FeatureId,
    pub sweep_feature_id: FeatureId,
    pub profile_bounds_bits: [u64; 4],
    pub path_bits: [u64; 4],
    pub canonical_input_digest: String,
}

impl ExactSweepRequest {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        let [profile_feature_id, path_feature_id, sweep_feature_id] = definition.feature_ids()
        else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        let profile = snapshot
            .feature(*profile_feature_id)
            .ok_or(ExactProductError::ProfileNotFound(*profile_feature_id))?;
        let path = snapshot
            .feature(*path_feature_id)
            .ok_or(ExactProductError::ProfileNotFound(*path_feature_id))?;
        let sweep = snapshot
            .feature(*sweep_feature_id)
            .ok_or(ExactProductError::UnsupportedDefinition)?;
        let FeatureKind::Profile { points_mm } = profile.kind() else {
            return Err(ExactProductError::UnsupportedProfile);
        };
        let FeatureKind::SegmentProfile {
            segments,
            closed: false,
        } = path.kind()
        else {
            return Err(ExactProductError::UnsupportedProfile);
        };
        let [ProfileSegment::Line { start_mm, end_mm }] = segments.as_slice() else {
            return Err(ExactProductError::UnsupportedProfile);
        };
        let FeatureKind::Sweep {
            profile: sweep_profile,
            path: sweep_path,
        } = sweep.kind()
        else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        if sweep_profile != profile_feature_id || sweep_path != path_feature_id {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let profile_bounds =
            rectangle_bounds(points_mm).ok_or(ExactProductError::UnsupportedProfile)?;
        let path_length = (end_mm[0] - start_mm[0]).hypot(end_mm[1] - start_mm[1]);
        if !path_length.is_finite() || path_length <= 1.0e-6 {
            return Err(ExactProductError::UnsupportedProfile);
        }
        let source_digest = snapshot.canonical_digest();
        let profile_bounds_bits = profile_bounds.map(f64::to_bits);
        let path_bits = [start_mm[0], start_mm[1], end_mm[0], end_mm[1]].map(f64::to_bits);
        let canonical_input_digest = digest(&format!(
            "{}:{}:{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
            EXACT_SWEEP_SCHEMA_V1,
            snapshot.document_id().0,
            snapshot.revision_id(),
            definition_id.0,
            profile_feature_id.0,
            path_feature_id.0,
            sweep_feature_id.0,
            profile_bounds_bits[0],
            profile_bounds_bits[1],
            profile_bounds_bits[2],
            profile_bounds_bits[3],
            path_bits[0],
            path_bits[1],
            path_bits[2],
            path_bits[3],
            source_digest,
        ));
        Ok(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id: *profile_feature_id,
            path_feature_id: *path_feature_id,
            sweep_feature_id: *sweep_feature_id,
            profile_bounds_bits,
            path_bits,
            canonical_input_digest,
        })
    }

    #[must_use]
    pub fn profile_bounds_mm(&self) -> [f64; 4] {
        self.profile_bounds_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn path_mm(&self) -> [[f64; 2]; 2] {
        let values = self.path_bits.map(f64::from_bits);
        [[values[0], values[1]], [values[2], values[3]]]
    }

    #[must_use]
    pub fn path_length_mm(&self) -> f64 {
        let [start, end] = self.path_mm();
        (end[0] - start[0]).hypot(end[1] - start[1])
    }

    #[must_use]
    pub fn expected_vertices_mm(&self) -> [[f64; 3]; 8] {
        let [min_u, min_v, max_u, max_v] = self.profile_bounds_mm();
        let [start, end] = self.path_mm();
        let path_x = end[0] - start[0];
        let path_y = end[1] - start[1];
        let path_length = self.path_length_mm();
        let section = [path_y / path_length, -path_x / path_length];
        let point = |u: f64, v: f64, at_end: bool| {
            let along = if at_end { [path_x, path_y] } else { [0.0, 0.0] };
            [
                start[0] + section[0] * u + along[0],
                start[1] + section[1] * u + along[1],
                v,
            ]
        };
        [
            point(min_u, min_v, false),
            point(max_u, min_v, false),
            point(max_u, max_v, false),
            point(min_u, max_v, false),
            point(min_u, min_v, true),
            point(max_u, min_v, true),
            point(max_u, max_v, true),
            point(min_u, max_v, true),
        ]
    }

    #[must_use]
    pub fn expected_bounds_mm(&self) -> [[f64; 3]; 2] {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for point in self.expected_vertices_mm() {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        [min, max]
    }

    #[must_use]
    pub fn expected_volume_mm3(&self) -> f64 {
        let [min_u, min_v, max_u, max_v] = self.profile_bounds_mm();
        (max_u - min_u) * (max_v - min_v) * self.path_length_mm()
    }

    #[must_use]
    pub const fn producer_feature_id(&self) -> FeatureId {
        self.sweep_feature_id
    }

    #[must_use]
    pub const fn evaluator(&self) -> &'static str {
        EXACT_SWEEP_EVALUATOR_V1
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactSweepIdentity {
    pub schema: String,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub path_feature_id: FeatureId,
    pub sweep_feature_id: FeatureId,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactSweepPackage {
    pub identity: ExactSweepIdentity,
    pub bounds_mm: [[f64; 3]; 2],
    pub volume_mm3: f64,
    pub vertices: Vec<ExactVertex>,
    pub triangles: Vec<ExactTriangle>,
    pub references: Vec<BodySubshapeRef>,
}

impl ExactSweepPackage {
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.document_id == snapshot.document_id()
            && self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && ExactSweepRequest::from_snapshot(snapshot, self.identity.definition_id)
                .is_ok_and(|request| self.validate_for_request(&request).is_ok())
    }

    pub fn validate_for_request(
        &self,
        request: &ExactSweepRequest,
    ) -> Result<(), ExactProductError> {
        let expected_roles = [
            ExactFaceRole::SweepStart,
            ExactFaceRole::SweepEnd,
            ExactFaceRole::SweepSide0,
            ExactFaceRole::SweepSide1,
            ExactFaceRole::SweepSide2,
            ExactFaceRole::SweepSide3,
        ];
        if self.identity.schema != EXACT_SWEEP_SCHEMA_V1
            || self.identity.document_id != request.document_id
            || self.identity.source_revision != request.source_revision
            || self.identity.source_digest != request.source_digest
            || self.identity.definition_id != request.definition_id
            || self.identity.profile_feature_id != request.profile_feature_id
            || self.identity.path_feature_id != request.path_feature_id
            || self.identity.sweep_feature_id != request.sweep_feature_id
            || self.identity.canonical_input_digest != request.canonical_input_digest
            || self.identity.evaluator != request.evaluator()
            || self.identity.exact_input_digest.is_empty()
            || self.identity.result_fingerprint.is_empty()
            || self.identity.backend.is_empty()
            || self.identity.tolerance.is_empty()
            || self.bounds_mm != request.expected_bounds_mm()
            || (self.volume_mm3 - request.expected_volume_mm3()).abs() > 1.0e-6
            || self.vertices.len() != 8
            || self
                .vertices
                .iter()
                .zip(request.expected_vertices_mm())
                .any(|(actual, expected)| actual.position_mm != expected)
            || self.triangles.len() != 12
            || self.references.len() != expected_roles.len()
            || self
                .references
                .iter()
                .zip(expected_roles)
                .any(|(reference, role)| {
                    reference.role() != Some(role)
                        || !reference.matches_sweep_request(request)
                        || reference.exact_input_digest != self.identity.exact_input_digest
                        || reference.result_fingerprint != self.identity.result_fingerprint
                        || reference.backend != self.identity.backend
                        || reference.tolerance != self.identity.tolerance
                })
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactLoftSection {
    pub profile_feature_id: FeatureId,
    pub elevation_bits: u64,
    pub control_point_bits: Vec<[u64; 2]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactLoftRequest {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub loft_feature_id: FeatureId,
    pub sections: Vec<ExactLoftSection>,
    pub canonical_input_digest: String,
}

impl ExactLoftRequest {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        let lofts = definition
            .feature_ids()
            .iter()
            .filter_map(|feature_id| {
                let feature = snapshot.feature(*feature_id)?;
                let FeatureKind::Loft { sections } = feature.kind() else {
                    return None;
                };
                Some((*feature_id, sections))
            })
            .collect::<Vec<_>>();
        let [(loft_feature_id, loft_sections)] = lofts.as_slice() else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        if definition.feature_ids().last() != Some(loft_feature_id) {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let sections = loft_sections
            .iter()
            .map(|section| {
                let profile = snapshot
                    .feature(section.profile)
                    .ok_or(ExactProductError::ProfileNotFound(section.profile))?;
                let FeatureKind::SplineProfile { control_points_mm } = profile.kind() else {
                    return Err(ExactProductError::UnsupportedProfile);
                };
                Ok(ExactLoftSection {
                    profile_feature_id: section.profile,
                    elevation_bits: section.elevation_mm.to_bits(),
                    control_point_bits: control_points_mm
                        .iter()
                        .map(|point| point.map(f64::to_bits))
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>, ExactProductError>>()?;
        if !(2..=16).contains(&sections.len())
            || sections.iter().any(|section| {
                !(4..=MAX_EXACT_BREP_LOFT_CONTROL_POINTS)
                    .contains(&section.control_point_bits.len())
            })
        {
            return Err(ExactProductError::UnsupportedProfile);
        }
        let source_digest = snapshot.canonical_digest();
        let mut canonical = format!(
            "{}:{}:{}:{}:{}:{}",
            EXACT_LOFT_SCHEMA_V1,
            snapshot.document_id().0,
            snapshot.revision_id(),
            definition_id.0,
            loft_feature_id.0,
            source_digest,
        );
        for section in &sections {
            write!(
                canonical,
                ":{}:{:016x}:{}",
                section.profile_feature_id.0,
                section.elevation_bits,
                section.control_point_bits.len()
            )
            .unwrap();
            for point in &section.control_point_bits {
                write!(canonical, ":{:016x}:{:016x}", point[0], point[1]).unwrap();
            }
        }
        Ok(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            loft_feature_id: *loft_feature_id,
            sections,
            canonical_input_digest: digest(&canonical),
        })
    }

    #[must_use]
    pub fn protocol_values(&self) -> Vec<f64> {
        let mut values = Vec::new();
        values.push(self.sections.len() as f64);
        for section in &self.sections {
            values.push(section.control_point_bits.len() as f64);
            values.push(f64::from_bits(section.elevation_bits));
            values.extend(
                section
                    .control_point_bits
                    .iter()
                    .flat_map(|point| point.map(f64::from_bits)),
            );
        }
        values
    }

    #[must_use]
    pub fn control_point_count(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.control_point_bits.len())
            .sum()
    }

    #[must_use]
    pub const fn producer_feature_id(&self) -> FeatureId {
        self.loft_feature_id
    }

    #[must_use]
    pub const fn evaluator(&self) -> &'static str {
        EXACT_LOFT_EVALUATOR_V1
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactLoftIdentity {
    pub schema: String,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub loft_feature_id: FeatureId,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactLoftPackage {
    pub identity: ExactLoftIdentity,
    pub bounds_mm: [[f64; 3]; 2],
    pub volume_mm3: f64,
    pub topology_counts: [u32; 5],
    pub references: Vec<BodySubshapeRef>,
}

impl ExactLoftPackage {
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.document_id == snapshot.document_id()
            && self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && ExactLoftRequest::from_snapshot(snapshot, self.identity.definition_id)
                .is_ok_and(|request| self.validate_for_request(&request).is_ok())
    }

    pub fn validate_for_request(
        &self,
        request: &ExactLoftRequest,
    ) -> Result<(), ExactProductError> {
        let expected_roles = [
            ExactFaceRole::LoftStart,
            ExactFaceRole::LoftEnd,
            ExactFaceRole::LoftSide,
        ];
        let first_z = f64::from_bits(request.sections.first().unwrap().elevation_bits);
        let last_z = f64::from_bits(request.sections.last().unwrap().elevation_bits);
        let [min, max] = self.bounds_mm;
        let contains_inputs = request.sections.iter().all(|section| {
            let z = f64::from_bits(section.elevation_bits);
            section.control_point_bits.iter().all(|point| {
                let [x, y] = point.map(f64::from_bits);
                x >= min[0] - 1.0e-6
                    && x <= max[0] + 1.0e-6
                    && y >= min[1] - 1.0e-6
                    && y <= max[1] + 1.0e-6
                    && z >= min[2] - 1.0e-6
                    && z <= max[2] + 1.0e-6
            })
        });
        if self.identity.schema != EXACT_LOFT_SCHEMA_V1
            || self.identity.document_id != request.document_id
            || self.identity.source_revision != request.source_revision
            || self.identity.source_digest != request.source_digest
            || self.identity.definition_id != request.definition_id
            || self.identity.loft_feature_id != request.loft_feature_id
            || self.identity.canonical_input_digest != request.canonical_input_digest
            || self.identity.evaluator != request.evaluator()
            || self.identity.exact_input_digest.is_empty()
            || self.identity.result_fingerprint.is_empty()
            || self.identity.backend.is_empty()
            || self.identity.tolerance.is_empty()
            || min.into_iter().chain(max).any(|value| !value.is_finite())
            || (min[2] - first_z).abs() > 1.0e-6
            || (max[2] - last_z).abs() > 1.0e-6
            || !contains_inputs
            || !self.volume_mm3.is_finite()
            || self.volume_mm3 <= 0.0
            || self.topology_counts[2..] != [3, 1, 1]
            || self.references.len() != expected_roles.len()
            || self
                .references
                .iter()
                .zip(expected_roles)
                .any(|(reference, role)| {
                    reference.role() != Some(role)
                        || !reference.matches_loft_request(request)
                        || reference.exact_input_digest != self.identity.exact_input_digest
                        || reference.result_fingerprint != self.identity.result_fingerprint
                        || reference.backend != self.identity.backend
                        || reference.tolerance != self.identity.tolerance
                })
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactFeatureChainRequest {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub extrusion_feature_id: FeatureId,
    pub width_bits: u64,
    pub depth_bits: u64,
    pub height_bits: u64,
    pub circle: Option<ExactCircleProfile>,
    pub mixed_profile: Option<ExactMixedProfile>,
    pub pocket_depth_bits: Option<u64>,
    pub workplane_frame_bits: Option<[u64; 12]>,
    pub boolean: Option<ExactBooleanRequest>,
    pub shell: Option<ExactBoxShellRequest>,
    pub canonical_input_digest: String,
    pub legacy_canonical_input_digest: String,
}

impl ExactFeatureChainRequest {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Self, ExactProductError> {
        Self::from_scoped_snapshot(snapshot, definition_id, None)
    }

    fn from_scoped_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        producer_feature_id: Option<FeatureId>,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        let feature_ids = if let Some(producer_feature_id) = producer_feature_id {
            if snapshot.feature_is_suppressed(producer_feature_id) {
                return Err(ExactProductError::UnsupportedDefinition);
            }
            let graph = snapshot
                .feature_dependency_graph()
                .map_err(|_| ExactProductError::UnsupportedDefinition)?;
            let mut relevant = BTreeSet::from([producer_feature_id]);
            let mut pending = vec![producer_feature_id];
            while let Some(feature_id) = pending.pop() {
                let dependencies = graph
                    .dependencies(feature_id)
                    .ok_or(ExactProductError::UnsupportedDefinition)?;
                for dependency in dependencies {
                    if relevant.insert(*dependency) {
                        pending.push(*dependency);
                    }
                }
            }
            definition
                .feature_ids()
                .iter()
                .copied()
                .filter(|feature_id| {
                    relevant.contains(feature_id) && !snapshot.feature_is_suppressed(*feature_id)
                })
                .collect::<Vec<_>>()
        } else {
            definition
                .feature_ids()
                .iter()
                .copied()
                .filter(|feature_id| !snapshot.feature_is_suppressed(*feature_id))
                .collect()
        };
        if feature_ids.iter().any(|feature_id| {
            snapshot
                .feature(*feature_id)
                .is_some_and(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
        }) {
            let graph = snapshot
                .feature_dependency_graph()
                .map_err(|_| ExactProductError::UnsupportedDefinition)?;
            let producers = feature_ids
                .iter()
                .copied()
                .filter(|feature_id| {
                    snapshot.feature(*feature_id).is_some_and(|feature| {
                        matches!(
                            feature.kind(),
                            FeatureKind::Pad(_) | FeatureKind::SketchPocket(_)
                        )
                    }) && graph.dependents(*feature_id).is_some_and(|dependents| {
                        dependents.iter().all(|dependent| {
                            snapshot
                                .feature(*dependent)
                                .is_none_or(|feature| !feature.kind().produces_body())
                        })
                    })
                })
                .collect::<Vec<_>>();
            let [producer] = producers.as_slice() else {
                return Err(ExactProductError::UnsupportedDefinition);
            };
            return Self::from_pad_pocket_snapshot(snapshot, definition_id, *producer);
        }
        if feature_ids.iter().any(|feature_id| {
            snapshot.feature(*feature_id).is_some_and(|feature| {
                matches!(
                    feature.kind(),
                    FeatureKind::BottleProfileControl { .. } | FeatureKind::Revolve { .. }
                )
            })
        }) {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let extrusions = feature_ids
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::Extrusion { profile, height } = feature.kind() else {
                    return None;
                };
                Some((*id, *profile, height.millimetres()))
            })
            .collect::<Vec<_>>();
        let legacy_cuts = feature_ids
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::ThroughCut { target, profile } = feature.kind() else {
                    return None;
                };
                Some((*id, *target, *profile))
            })
            .collect::<Vec<_>>();
        let legacy_through_cut = !legacy_cuts.is_empty();
        let pockets = feature_ids
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::Pocket {
                    target,
                    profile,
                    depth,
                } = feature.kind()
                else {
                    return None;
                };
                Some((*id, *target, *profile, depth.millimetres()))
            })
            .collect::<Vec<_>>();
        let booleans = feature_ids
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::Boolean {
                    operation,
                    target,
                    tool,
                } = feature.kind()
                else {
                    return None;
                };
                Some((*id, *operation, *target, *tool))
            })
            .collect::<Vec<_>>();
        let shells = feature_ids
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::Shell {
                    target,
                    removed_faces,
                    thickness,
                } = feature.kind()
                else {
                    return None;
                };
                Some((*id, *target, removed_faces, thickness.millimetres()))
            })
            .collect::<Vec<_>>();
        let finishes = feature_ids
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::BottleEdgeFinish {
                    target,
                    edges,
                    kind,
                    amount,
                } = feature.kind()
                else {
                    return None;
                };
                Some((*id, *target, edges, *kind, amount.millimetres()))
            })
            .collect::<Vec<_>>();
        let (extrusion_feature_id, profile_feature_id, height_mm, boolean_source, pocket_depth_mm) =
            match (
                legacy_cuts.as_slice(),
                pockets.as_slice(),
                booleans.as_slice(),
            ) {
                ([], [], []) => {
                    let [(extrusion, profile, height)] = extrusions.as_slice() else {
                        return Err(ExactProductError::UnsupportedDefinition);
                    };
                    (*extrusion, *profile, *height, None, None)
                }
                ([(feature_id, target, profile)], [], []) => {
                    let [(extrusion, base_profile, height)] = extrusions.as_slice() else {
                        return Err(ExactProductError::UnsupportedDefinition);
                    };
                    if target != extrusion {
                        return Err(ExactProductError::UnsupportedDefinition);
                    }
                    (
                        *extrusion,
                        *base_profile,
                        *height,
                        Some((
                            *feature_id,
                            BooleanOperation::Cut,
                            *target,
                            *profile,
                            *profile,
                            *height,
                        )),
                        None,
                    )
                }
                ([], [(feature_id, target, profile, pocket_depth)], []) => {
                    let [(extrusion, base_profile, height)] = extrusions.as_slice() else {
                        return Err(ExactProductError::UnsupportedDefinition);
                    };
                    if target != extrusion || *pocket_depth <= 0.0 || *pocket_depth >= *height {
                        return Err(ExactProductError::UnsupportedThroughCut);
                    }
                    (
                        *extrusion,
                        *base_profile,
                        *height,
                        Some((
                            *feature_id,
                            BooleanOperation::Cut,
                            *target,
                            *profile,
                            *profile,
                            *height,
                        )),
                        Some(*pocket_depth),
                    )
                }
                ([], [], [(feature_id, operation, target, tool)]) => {
                    if extrusions.len() != 2 {
                        return Err(ExactProductError::UnsupportedBoolean(*operation));
                    }
                    let (target_id, target_profile, target_height) = extrusions
                        .iter()
                        .find(|(id, _, _)| id == target)
                        .copied()
                        .ok_or(ExactProductError::UnsupportedDefinition)?;
                    let (tool_id, tool_profile, tool_height) = extrusions
                        .iter()
                        .find(|(id, _, _)| id == tool)
                        .copied()
                        .ok_or(ExactProductError::UnsupportedDefinition)?;
                    if target_height.to_bits() != tool_height.to_bits() {
                        return Err(ExactProductError::UnsupportedBoolean(*operation));
                    }
                    (
                        target_id,
                        target_profile,
                        target_height,
                        Some((
                            *feature_id,
                            *operation,
                            target_id,
                            tool_id,
                            tool_profile,
                            tool_height,
                        )),
                        None,
                    )
                }
                _ => return Err(ExactProductError::UnsupportedDefinition),
            };
        let shell = match (shells.as_slice(), finishes.as_slice()) {
            ([], []) => None,
            ([(shell_id, target, removed_faces, thickness)], [])
                if *target == extrusion_feature_id
                    && removed_faces.len() == 1
                    && removed_faces[0].as_str() == "extrusion.top"
                    && thickness.is_finite()
                    && *thickness > 0.0 =>
            {
                Some(ExactBoxShellRequest {
                    shell_feature_id: *shell_id,
                    thickness_bits: thickness.to_bits(),
                    edge_finish_feature_id: None,
                    edge_finish_kind: None,
                    edge_finish_amount_bits: None,
                })
            }
            (
                [(shell_id, target, removed_faces, thickness)],
                [(finish_id, finish_target, edges, kind, amount)],
            ) if *target == extrusion_feature_id
                && *finish_target == *shell_id
                && removed_faces.len() == 1
                && removed_faces[0].as_str() == "extrusion.top"
                && edges.len() == 1
                && edges[0].as_str() == "shell.edge.top-east"
                && thickness.is_finite()
                && *thickness > 0.0
                && amount.is_finite()
                && *amount > 0.0 =>
            {
                Some(ExactBoxShellRequest {
                    shell_feature_id: *shell_id,
                    thickness_bits: thickness.to_bits(),
                    edge_finish_feature_id: Some(*finish_id),
                    edge_finish_kind: Some(*kind),
                    edge_finish_amount_bits: Some(amount.to_bits()),
                })
            }
            _ => return Err(ExactProductError::UnsupportedDefinition),
        };
        if shell.is_some()
            && (boolean_source.is_some() || pocket_depth_mm.is_some() || extrusions.len() != 1)
        {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let profile = snapshot
            .feature(profile_feature_id)
            .ok_or(ExactProductError::ProfileNotFound(profile_feature_id))?;
        let (width_mm, depth_mm, circle, mixed_profile) = match profile.kind() {
            FeatureKind::Profile { points_mm } => {
                let (width, depth) = origin_rectangle_size(points_mm)
                    .ok_or(ExactProductError::UnsupportedProfile)?;
                (width, depth, None, None)
            }
            FeatureKind::SegmentProfile { segments, closed } => {
                if let Some(circle) = exact_circle_profile(segments, *closed) {
                    let diameter = 2.0 * f64::from_bits(circle.radius_bits);
                    (diameter, diameter, Some(circle), None)
                } else {
                    let mixed = exact_mixed_profile(segments, *closed)
                        .ok_or(ExactProductError::UnsupportedProfile)?;
                    let width =
                        f64::from_bits(mixed.bounds_bits[2]) - f64::from_bits(mixed.bounds_bits[0]);
                    let depth =
                        f64::from_bits(mixed.bounds_bits[3]) - f64::from_bits(mixed.bounds_bits[1]);
                    (width, depth, None, Some(mixed))
                }
            }
            _ => return Err(ExactProductError::UnsupportedProfile),
        };
        if !height_mm.is_finite() || height_mm <= 0.0 {
            return Err(ExactProductError::UnsupportedExtrusion);
        }
        if let Some(shell) = &shell {
            let thickness = f64::from_bits(shell.thickness_bits);
            if circle.is_some()
                || mixed_profile.is_some()
                || thickness * 2.0 >= width_mm.min(depth_mm)
                || thickness >= height_mm
                || shell
                    .edge_finish_amount_bits
                    .is_some_and(|bits| f64::from_bits(bits) >= width_mm.min(depth_mm) * 0.5)
            {
                return Err(ExactProductError::UnsupportedShell);
            }
        }
        let boolean = boolean_source
            .map(
                |(feature_id, operation, target, tool, tool_profile_id, tool_height)| {
                    let tool_profile = snapshot
                        .feature(tool_profile_id)
                        .ok_or(ExactProductError::ProfileNotFound(tool_profile_id))?;
                    if circle.is_some() || mixed_profile.is_some() {
                        return Err(ExactProductError::UnsupportedBoolean(operation));
                    }
                    let ([min_x, min_y, max_x, max_y], tool_circle, tool_linear_profile) =
                        match tool_profile.kind() {
                            FeatureKind::Profile { points_mm } => (
                                rectangle_bounds(points_mm)
                                    .ok_or(ExactProductError::UnsupportedBoolean(operation))?,
                                None,
                                None,
                            ),
                            FeatureKind::SegmentProfile { segments, closed } => {
                                if let Some(circle) = exact_circle_profile(segments, *closed) {
                                    let center_x = f64::from_bits(circle.center_x_bits);
                                    let center_y = f64::from_bits(circle.center_y_bits);
                                    let radius = f64::from_bits(circle.radius_bits);
                                    (
                                        [
                                            center_x - radius,
                                            center_y - radius,
                                            center_x + radius,
                                            center_y + radius,
                                        ],
                                        Some(circle),
                                        None,
                                    )
                                } else {
                                    let profile = exact_mixed_profile(segments, *closed)
                                        .ok_or(ExactProductError::UnsupportedBoolean(operation))?;
                                    let bounds = profile.bounds_bits.map(f64::from_bits);
                                    let rectangle = line_rectangle_bounds(segments);
                                    (
                                        rectangle.unwrap_or(bounds),
                                        None,
                                        rectangle.is_none().then_some(profile),
                                    )
                                }
                            }
                            _ => return Err(ExactProductError::UnsupportedProfile),
                        };
                    let supported = match operation {
                        BooleanOperation::Cut => {
                            let contained = min_x > 1.0e-6
                                && min_y > 1.0e-6
                                && max_x < width_mm - 1.0e-6
                                && max_y < depth_mm - 1.0e-6;
                            tool_linear_profile.as_ref().map_or(
                                contained
                                    || tool_circle.is_some_and(|circle| {
                                        circle.side_overlap(width_mm, depth_mm).is_some()
                                            || circle.corner_overlap(width_mm, depth_mm).is_some()
                                            || circle
                                                .outside_side_overlap(width_mm, depth_mm)
                                                .is_some()
                                            || circle
                                                .center_on_side_overlap(width_mm, depth_mm)
                                                .is_some()
                                            || circle
                                                .center_on_corner_overlap(width_mm, depth_mm)
                                                .is_some()
                                            || circle
                                                .outside_corner_overlap(width_mm, depth_mm)
                                                .is_some()
                                    }),
                                |profile| {
                                (contained
                                    && (profile.has_only_line_segments()
                                        || profile.is_line_arc_d_profile()
                                        || profile.is_line_arc_capsule_profile()
                                        || profile.is_line_arc_rounded_rectangle_profile()
                                        || profile.is_strict_convex_line_arc_profile()))
                                    || profile
                                        .d_profile_arc_only_clipped_side_overlap(width_mm, depth_mm)
                                        .is_some()
                                    || profile
                                        .capsule_side_overlap(width_mm, depth_mm)
                                        .is_some()
                                    || profile
                                        .capsule_corner_overlap(width_mm, depth_mm)
                                        .is_some()
                                    || profile
                                        .rounded_rectangle_chord_side_overlap(width_mm, depth_mm)
                                        .is_some()
                                    || profile
                                        .strict_convex_line_arc_clipped_side_overlap(
                                            width_mm, depth_mm,
                                        )
                                        .is_some()
                                    || profile
                                        .strict_convex_line_clipped_side_overlap(width_mm, depth_mm)
                                        .is_some()
                                    || profile
                                        .strict_convex_arc_only_clipped_side_overlap(
                                            width_mm, depth_mm,
                                        )
                                        .is_some()
                                    || profile
                                        .strict_convex_line_arc_clipped_south_east_corner_overlap(
                                            width_mm, depth_mm,
                                        )
                                        .is_some()
                                    || profile
                                        .strict_convex_line_arc_clipped_south_west_corner_overlap(
                                            width_mm, depth_mm,
                                        )
                                        .is_some()
                                    || profile
                                        .strict_convex_line_arc_clipped_north_east_corner_overlap(
                                            width_mm, depth_mm,
                                        )
                                        .is_some()
                                    || profile
                                        .strict_convex_line_arc_clipped_north_west_corner_overlap(
                                            width_mm, depth_mm,
                                        )
                                        .is_some()
                                    || profile
                                        .rounded_rectangle_side_overlap_area(width_mm, depth_mm)
                                        .is_some()
                                    || profile
                                        .rounded_rectangle_corner_overlap_area(width_mm, depth_mm)
                                        .is_some()
                                    || profile
                                        .rounded_rectangle_arc_clipped_corner_overlap_area(
                                            width_mm, depth_mm,
                                        )
                                        .is_some()
                                    || profile
                                        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                                            width_mm, depth_mm,
                                        )
                                        .is_some()
                            })
                        }
                        BooleanOperation::Union if tool_circle.is_some() => tool_circle
                            .is_some_and(|circle| {
                                circle_contains_rectangle(circle, width_mm, depth_mm)
                                    || circle.side_overlap(width_mm, depth_mm).is_some()
                                    || circle.corner_overlap(width_mm, depth_mm).is_some()
                                    || circle.outside_side_overlap(width_mm, depth_mm).is_some()
                                    || circle.center_on_side_overlap(width_mm, depth_mm).is_some()
                                    || circle.center_on_corner_overlap(width_mm, depth_mm).is_some()
                                    || circle.outside_corner_overlap(width_mm, depth_mm).is_some()
                            }),
                        BooleanOperation::Union => tool_linear_profile.as_ref().map_or_else(
                            || {
                                rectangular_union_bounds(
                                    width_mm,
                                    depth_mm,
                                    [min_x, min_y, max_x, max_y],
                                )
                                .is_some()
                            },
                            |profile| {
                                (profile.has_only_line_segments()
                                    || profile.is_line_arc_d_profile()
                                    || profile.is_line_arc_capsule_profile()
                                    || profile.is_line_arc_rounded_rectangle_profile()
                                    || profile.is_strict_convex_line_arc_profile())
                                    && (polygon_contains_rectangle(profile, width_mm, depth_mm)
                                        || profile
                                            .d_profile_arc_only_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .capsule_side_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .capsule_corner_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_chord_side_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .strict_convex_line_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_arc_only_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_south_east_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_south_west_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_north_east_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_north_west_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_side_overlap_area(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_arc_clipped_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some())
                            },
                        ),
                        BooleanOperation::Intersect if tool_circle.is_some() => {
                            min_x > 1.0e-6
                                && min_y > 1.0e-6
                                && max_x < width_mm - 1.0e-6
                                && max_y < depth_mm - 1.0e-6
                                || tool_circle.is_some_and(|circle| {
                                    circle.side_overlap(width_mm, depth_mm).is_some()
                                        || circle.corner_overlap(width_mm, depth_mm).is_some()
                                        || circle
                                            .outside_side_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || circle
                                            .center_on_side_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || circle
                                            .center_on_corner_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || circle
                                            .outside_corner_overlap(width_mm, depth_mm)
                                            .is_some()
                                })
                        }
                        BooleanOperation::Intersect => tool_linear_profile.as_ref().map_or_else(
                            || {
                                rectangular_intersection_bounds(
                                    width_mm,
                                    depth_mm,
                                    [min_x, min_y, max_x, max_y],
                                )
                                .is_some()
                            },
                            |profile| {
                                (profile.has_only_line_segments()
                                    || profile.is_line_arc_d_profile()
                                    || profile.is_line_arc_capsule_profile()
                                    || profile.is_line_arc_rounded_rectangle_profile()
                                    || profile.is_strict_convex_line_arc_profile())
                                    && (polygon_within_rectangle(profile, width_mm, depth_mm)
                                        || profile
                                            .d_profile_arc_only_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .capsule_side_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .capsule_corner_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_chord_side_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .strict_convex_line_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_arc_only_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_south_east_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_south_west_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_north_east_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_north_west_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_side_overlap_area(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_arc_clipped_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some())
                            },
                        ),
                        BooleanOperation::Split if tool_circle.is_some() => {
                            min_x > 1.0e-6
                                && min_y > 1.0e-6
                                && max_x < width_mm - 1.0e-6
                                && max_y < depth_mm - 1.0e-6
                                || tool_circle.is_some_and(|circle| {
                                    circle.side_overlap(width_mm, depth_mm).is_some()
                                        || circle.corner_overlap(width_mm, depth_mm).is_some()
                                        || circle.outside_side_overlap(width_mm, depth_mm).is_some()
                                        || circle.center_on_side_overlap(width_mm, depth_mm).is_some()
                                        || circle.center_on_corner_overlap(width_mm, depth_mm).is_some()
                                        || circle.outside_corner_overlap(width_mm, depth_mm).is_some()
                                })
                        }
                        BooleanOperation::Split => tool_linear_profile.as_ref().map_or_else(
                            || {
                                rectangular_split_supported(
                                    width_mm,
                                    depth_mm,
                                    [min_x, min_y, max_x, max_y],
                                )
                            },
                            |profile| {
                                (profile.has_only_line_segments()
                                    || profile.is_line_arc_d_profile()
                                    || profile.is_line_arc_capsule_profile()
                                    || profile.is_line_arc_rounded_rectangle_profile()
                                    || profile.is_strict_convex_line_arc_profile())
                                    && (polygon_within_rectangle(profile, width_mm, depth_mm)
                                        || profile
                                            .d_profile_arc_only_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .capsule_side_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .capsule_corner_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_chord_side_overlap(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .strict_convex_line_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_arc_only_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_side_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_south_east_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_south_west_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_north_east_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .strict_convex_line_arc_clipped_north_west_corner_overlap(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_side_overlap_area(width_mm, depth_mm)
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_arc_clipped_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some()
                                        || profile
                                            .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                                                width_mm, depth_mm,
                                            )
                                            .is_some())
                            },
                        ),
                    };
                    if tool_height.to_bits() != height_mm.to_bits() || !supported {
                        return Err(ExactProductError::UnsupportedBoolean(operation));
                    }
                    Ok(ExactBooleanRequest {
                        feature_id,
                        operation,
                        target_feature_id: target,
                        tool_feature_id: tool,
                        profile_feature_id: tool_profile_id,
                        min_x_bits: min_x.to_bits(),
                        min_y_bits: min_y.to_bits(),
                        width_bits: (max_x - min_x).to_bits(),
                        depth_bits: (max_y - min_y).to_bits(),
                        circle: tool_circle,
                        profile: tool_linear_profile,
                    })
                },
            )
            .transpose()?;
        let source_digest = snapshot.canonical_digest();
        let canonical_input = format!(
            "{}:{}:{}:{}:{:016x}:{:016x}:{:016x}",
            EXACT_PRODUCT_SCHEMA_V1,
            snapshot.document_id().0,
            definition_id.0,
            extrusion_feature_id.0,
            width_mm.to_bits(),
            depth_mm.to_bits(),
            height_mm.to_bits()
        );
        let legacy_canonical_input = format!(
            "{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{}",
            EXACT_PRODUCT_SCHEMA_V1,
            snapshot.document_id().0,
            snapshot.revision_id(),
            definition_id.0,
            extrusion_feature_id.0,
            width_mm.to_bits(),
            depth_mm.to_bits(),
            height_mm.to_bits(),
            source_digest
        );
        let initial_input_digest = |input: &str| {
            boolean.as_ref().map_or_else(
                || digest(input),
                |cut| {
                    if legacy_through_cut {
                        digest(&format!(
                            "{input}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}",
                            cut.feature_id.0,
                            cut.profile_feature_id.0,
                            cut.min_x_bits,
                            cut.min_y_bits,
                            cut.width_bits,
                            cut.depth_bits
                        ))
                    } else {
                        digest(&format!(
                            "{input}:{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}",
                            cut.feature_id.0,
                            match cut.operation {
                                BooleanOperation::Cut => "cut",
                                BooleanOperation::Union => "union",
                                BooleanOperation::Intersect => "intersect",
                                BooleanOperation::Split => "split",
                            },
                            cut.target_feature_id.0,
                            cut.tool_feature_id.0,
                            cut.profile_feature_id.0,
                            cut.min_x_bits,
                            cut.min_y_bits,
                            cut.width_bits,
                            cut.depth_bits
                        ))
                    }
                },
            )
        };
        let mut canonical_input_digest = initial_input_digest(&canonical_input);
        let mut legacy_canonical_input_digest = initial_input_digest(&legacy_canonical_input);
        if let Some(circle) = circle {
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:circle:{:016x}:{:016x}:{:016x}:{}",
                circle.center_x_bits, circle.center_y_bits, circle.radius_bits, circle.clockwise
            ));
            legacy_canonical_input_digest = digest(&format!(
                "{legacy_canonical_input_digest}:circle:{:016x}:{:016x}:{:016x}:{}",
                circle.center_x_bits, circle.center_y_bits, circle.radius_bits, circle.clockwise
            ));
        }
        if let Some(mixed) = &mixed_profile {
            let mut exact_segments = String::new();
            for segment in &mixed.segments {
                match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => write!(
                        exact_segments,
                        ":L:{:016x}:{:016x}:{:016x}:{:016x}",
                        start_bits[0], start_bits[1], end_bits[0], end_bits[1]
                    )
                    .expect("writing to String cannot fail"),
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => write!(
                        exact_segments,
                        ":A:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                        start_bits[0],
                        start_bits[1],
                        end_bits[0],
                        end_bits[1],
                        center_bits[0],
                        center_bits[1],
                        clockwise
                    )
                    .expect("writing to String cannot fail"),
                }
            }
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:mixed{}:{:016x}",
                exact_segments, mixed.area_bits
            ));
            legacy_canonical_input_digest = digest(&format!(
                "{legacy_canonical_input_digest}:mixed{}:{:016x}",
                exact_segments, mixed.area_bits
            ));
        }
        if let Some(tool_circle) = boolean.as_ref().and_then(|boolean| boolean.circle) {
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:cut-circle:{:016x}:{:016x}:{:016x}:{}",
                tool_circle.center_x_bits,
                tool_circle.center_y_bits,
                tool_circle.radius_bits,
                tool_circle.clockwise
            ));
            legacy_canonical_input_digest = digest(&format!(
                "{legacy_canonical_input_digest}:cut-circle:{:016x}:{:016x}:{:016x}:{}",
                tool_circle.center_x_bits,
                tool_circle.center_y_bits,
                tool_circle.radius_bits,
                tool_circle.clockwise
            ));
        }
        if let Some(profile) = boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
        {
            let mut exact_segments = String::new();
            for segment in &profile.segments {
                match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => write!(
                        exact_segments,
                        ":L:{:016x}:{:016x}:{:016x}:{:016x}",
                        start_bits[0], start_bits[1], end_bits[0], end_bits[1]
                    )
                    .expect("writing to String cannot fail"),
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => write!(
                        exact_segments,
                        ":A:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                        start_bits[0],
                        start_bits[1],
                        end_bits[0],
                        end_bits[1],
                        center_bits[0],
                        center_bits[1],
                        u8::from(*clockwise)
                    )
                    .expect("writing to String cannot fail"),
                }
            }
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:cut-linear{}:{:016x}",
                exact_segments, profile.area_bits
            ));
            legacy_canonical_input_digest = digest(&format!(
                "{legacy_canonical_input_digest}:cut-linear{}:{:016x}",
                exact_segments, profile.area_bits
            ));
        }
        let pocket_depth_bits = pocket_depth_mm.map(f64::to_bits);
        if let Some(depth_bits) = pocket_depth_bits {
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:pocket:{depth_bits:016x}"
            ));
            legacy_canonical_input_digest = digest(&format!(
                "{legacy_canonical_input_digest}:pocket:{depth_bits:016x}"
            ));
        }
        if let Some(shell) = &shell {
            let finish_kind = shell.edge_finish_kind.map_or("none", |kind| match kind {
                EdgeFinishKind::Fillet => "fillet",
                EdgeFinishKind::Chamfer => "chamfer",
            });
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:shell:{}:{:016x}:finish:{}:{}:{:016x}",
                shell.shell_feature_id.0,
                shell.thickness_bits,
                shell.edge_finish_feature_id.map_or(0, |id| id.0),
                finish_kind,
                shell.edge_finish_amount_bits.unwrap_or(0),
            ));
            legacy_canonical_input_digest = digest(&format!(
                "{legacy_canonical_input_digest}:shell:{}:{:016x}:finish:{}:{}:{:016x}",
                shell.shell_feature_id.0,
                shell.thickness_bits,
                shell.edge_finish_feature_id.map_or(0, |id| id.0),
                finish_kind,
                shell.edge_finish_amount_bits.unwrap_or(0),
            ));
        }
        Ok(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id,
            extrusion_feature_id,
            width_bits: width_mm.to_bits(),
            depth_bits: depth_mm.to_bits(),
            height_bits: height_mm.to_bits(),
            circle,
            mixed_profile,
            pocket_depth_bits,
            workplane_frame_bits: None,
            boolean,
            shell,
            canonical_input_digest,
            legacy_canonical_input_digest,
        })
    }

    pub fn terminal_body_requests(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<BTreeMap<BodyId, Self>, ExactProductError> {
        exact_body_terminal_features(snapshot, definition_id)?
            .into_iter()
            .map(|(body_id, producer_feature_id)| {
                Self::from_snapshot_for_producer(snapshot, definition_id, producer_feature_id)
                    .map(|request| (body_id, request))
            })
            .collect()
    }

    pub fn from_snapshot_for_body(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        body_id: BodyId,
    ) -> Result<Self, ExactProductError> {
        let producer_feature_id = exact_body_terminal_features(snapshot, definition_id)?
            .get(&body_id)
            .copied()
            .ok_or(ExactProductError::BodyOutputNotFound {
                definition_id,
                body_id,
            })?;
        Self::from_snapshot_for_producer(snapshot, definition_id, producer_feature_id)
    }

    pub fn from_snapshot_for_producer(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
    ) -> Result<Self, ExactProductError> {
        if snapshot.feature_is_suppressed(producer_feature_id) {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let producer = snapshot
            .feature(producer_feature_id)
            .ok_or(ExactProductError::UnsupportedDefinition)?;
        let request = if matches!(
            producer.kind(),
            FeatureKind::Pad(_) | FeatureKind::SketchPocket(_)
        ) {
            Self::from_pad_pocket_snapshot(snapshot, definition_id, producer_feature_id)?
        } else {
            Self::from_scoped_snapshot(snapshot, definition_id, Some(producer_feature_id))?
        };
        if request.producer_feature_id() != producer_feature_id {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        Ok(request)
    }

    fn from_pad_pocket_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
    ) -> Result<Self, ExactProductError> {
        let producer = snapshot
            .feature(producer_feature_id)
            .filter(|feature| feature.definition_id() == definition_id)
            .ok_or(ExactProductError::UnsupportedDefinition)?;
        let (pad_id, pad, pocket) = match producer.kind() {
            FeatureKind::Pad(pad) => (producer_feature_id, pad, None),
            FeatureKind::SketchPocket(pocket) => {
                let pad = snapshot
                    .feature(pocket.target)
                    .and_then(|feature| match feature.kind() {
                        FeatureKind::Pad(spec) if feature.definition_id() == definition_id => {
                            Some(spec)
                        }
                        _ => None,
                    })
                    .ok_or(ExactProductError::UnsupportedDefinition)?;
                (
                    pocket.target,
                    pad,
                    Some((producer_feature_id, pocket.clone())),
                )
            }
            _ => return Err(ExactProductError::UnsupportedDefinition),
        };
        let pad_sketch = snapshot
            .feature(pad.sketch)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Sketch(spec) => Some(spec),
                _ => None,
            })
            .ok_or(ExactProductError::UnsupportedProfile)?;
        let pad_workplane = snapshot
            .feature(pad_sketch.workplane)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Workplane(spec) => Some(spec),
                _ => None,
            })
            .ok_or(ExactProductError::UnsupportedDefinition)?;
        let pad_region = selected_solved_region(pad_sketch, pad.region)?;
        let height_mm = match &pad.extent {
            crate::sketch::FeatureExtent::Blind(distance) => distance.millimetres(),
            _ => return Err(ExactProductError::UnsupportedDefinition),
        };
        let direction_sign = match pad.direction {
            FeatureDirection::AlongNormal => 1.0,
            FeatureDirection::OppositeNormal => -1.0,
            FeatureDirection::Vector(_) => return Err(ExactProductError::UnsupportedDefinition),
        };
        let ExactRegionProfile {
            width_mm,
            depth_mm,
            circle,
            mixed_profile,
            mut frame,
        } = exact_region_profile(pad_region, pad_workplane)?;
        let (boolean, pocket_depth_bits) = if let Some((feature_id, pocket)) = pocket {
            let pocket_depth_mm = match &pocket.extent {
                crate::sketch::FeatureExtent::Blind(distance) => distance.millimetres(),
                _ => return Err(ExactProductError::UnsupportedThroughCut),
            };
            let directions_are_opposite = matches!(
                (pad.direction, pocket.direction),
                (
                    FeatureDirection::AlongNormal,
                    FeatureDirection::OppositeNormal
                ) | (
                    FeatureDirection::OppositeNormal,
                    FeatureDirection::AlongNormal
                )
            );
            if pocket.target != pad_id
                || circle.is_some()
                || mixed_profile.is_some()
                || !directions_are_opposite
                || pocket_depth_mm >= height_mm
                || pocket.support.producer_feature_id != pad_id
            {
                return Err(ExactProductError::UnsupportedThroughCut);
            }
            let pocket_sketch = snapshot
                .feature(pocket.sketch)
                .and_then(|feature| match feature.kind() {
                    FeatureKind::Sketch(spec) => Some(spec),
                    _ => None,
                })
                .ok_or(ExactProductError::UnsupportedProfile)?;
            let pocket_workplane = snapshot
                .feature(pocket_sketch.workplane)
                .and_then(|feature| match feature.kind() {
                    FeatureKind::Workplane(spec) => Some(spec),
                    _ => None,
                })
                .ok_or(ExactProductError::UnsupportedDefinition)?;
            let support_is_resolved = matches!(
                &pocket_workplane.support,
                WorkplaneSupport::PlanarFace {
                    reference,
                    health: WorkplaneSupportHealth::Resolved,
                } if reference.as_ref() == pocket.support.as_ref()
            );
            if !support_is_resolved || !parallel_frame_axes(pad_workplane, pocket_workplane) {
                return Err(ExactProductError::UnsupportedDefinition);
            }
            let pocket_region = selected_solved_region(pocket_sketch, pocket.region)?;
            if !pocket_region.holes.is_empty() {
                return Err(ExactProductError::UnsupportedProfile);
            }
            let (min_x, min_y, max_x, max_y, pocket_circle, pocket_profile) = match &pocket_region
                .outer
            {
                SolvedSketchRegionProfile::Polyline(points)
                    if rectangle_bounds(points).is_some() =>
                {
                    let [min_x, min_y, max_x, max_y] =
                        rectangle_bounds(points).ok_or(ExactProductError::UnsupportedProfile)?;
                    (min_x, min_y, max_x, max_y, None, None)
                }
                SolvedSketchRegionProfile::Polyline(_) | SolvedSketchRegionProfile::Boundary(_) => {
                    let profile = exact_mixed_profile_from_solved(&pocket_region.outer)
                        .ok_or(ExactProductError::UnsupportedProfile)?;
                    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
                    (min_x, min_y, max_x, max_y, None, Some(profile))
                }
                SolvedSketchRegionProfile::Circle {
                    center_mm,
                    radius_mm,
                } => (
                    center_mm[0] - radius_mm,
                    center_mm[1] - radius_mm,
                    center_mm[0] + radius_mm,
                    center_mm[1] + radius_mm,
                    Some(ExactCircleProfile {
                        center_x_bits: center_mm[0].to_bits(),
                        center_y_bits: center_mm[1].to_bits(),
                        radius_bits: radius_mm.to_bits(),
                        clockwise: false,
                    }),
                    None,
                ),
            };
            if min_x <= 0.0 || min_y <= 0.0 || max_x >= width_mm || max_y >= depth_mm {
                return Err(ExactProductError::UnsupportedThroughCut);
            }
            (
                Some(ExactBooleanRequest {
                    feature_id,
                    operation: BooleanOperation::Cut,
                    target_feature_id: pad_id,
                    tool_feature_id: pocket.sketch,
                    profile_feature_id: pocket.sketch,
                    min_x_bits: min_x.to_bits(),
                    min_y_bits: min_y.to_bits(),
                    width_bits: (max_x - min_x).to_bits(),
                    depth_bits: (max_y - min_y).to_bits(),
                    circle: pocket_circle,
                    profile: pocket_profile,
                }),
                Some(pocket_depth_mm.to_bits()),
            )
        } else {
            (None, None)
        };
        frame[9] *= direction_sign;
        frame[10] *= direction_sign;
        frame[11] *= direction_sign;
        let frame_bits = frame.map(f64::to_bits);
        let source_digest = snapshot.canonical_digest();
        let mut canonical_input = format!(
            "{}:{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}",
            EXACT_PRODUCT_SCHEMA_V1,
            snapshot.document_id().0,
            definition_id.0,
            pad_sketch.workplane.0,
            pad.sketch.0,
            pad.region.0,
            width_mm.to_bits(),
            depth_mm.to_bits(),
            height_mm.to_bits(),
        );
        for bits in frame_bits {
            write!(canonical_input, ":{bits:016x}").expect("writing to String cannot fail");
        }
        if let Some(profile) = &mixed_profile {
            append_exact_mixed_profile_identity(&mut canonical_input, "pad-profile", profile);
        }
        if let Some(circle) = circle {
            write!(
                canonical_input,
                ":circle:{:016x}:{:016x}:{:016x}:{}",
                circle.center_x_bits, circle.center_y_bits, circle.radius_bits, circle.clockwise,
            )
            .expect("writing to String cannot fail");
        }
        if let Some(cut) = &boolean {
            write!(
                canonical_input,
                ":pocket:{}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                cut.feature_id.0,
                cut.profile_feature_id.0,
                cut.target_feature_id.0,
                cut.min_x_bits,
                cut.min_y_bits,
                cut.width_bits,
                cut.depth_bits,
                pocket_depth_bits.unwrap_or_default(),
            )
            .expect("writing to String cannot fail");
            if let Some(profile) = &cut.profile {
                append_exact_mixed_profile_identity(
                    &mut canonical_input,
                    "pocket-profile",
                    profile,
                );
            }
        }
        let canonical_input_digest = digest(&canonical_input);
        Ok(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id: pad.sketch,
            extrusion_feature_id: pad_id,
            width_bits: width_mm.to_bits(),
            depth_bits: depth_mm.to_bits(),
            height_bits: height_mm.to_bits(),
            circle,
            mixed_profile,
            pocket_depth_bits,
            workplane_frame_bits: Some(frame_bits),
            boolean,
            shell: None,
            canonical_input_digest: canonical_input_digest.clone(),
            legacy_canonical_input_digest: canonical_input_digest,
        })
    }

    #[must_use]
    pub fn dimensions_mm(&self) -> [f64; 3] {
        [
            f64::from_bits(self.width_bits),
            f64::from_bits(self.depth_bits),
            f64::from_bits(self.height_bits),
        ]
    }

    #[must_use]
    pub fn expected_bounds_mm(&self) -> [[f64; 3]; 2] {
        let [width, depth, height] = self.dimensions_mm();
        if let Some(circle) = self.circle {
            let center_x = f64::from_bits(circle.center_x_bits);
            let center_y = f64::from_bits(circle.center_y_bits);
            let radius = f64::from_bits(circle.radius_bits);
            return [
                [center_x - radius, center_y - radius, 0.0],
                [center_x + radius, center_y + radius, height],
            ];
        }
        if let Some(mixed) = &self.mixed_profile {
            return [
                [
                    f64::from_bits(mixed.bounds_bits[0]),
                    f64::from_bits(mixed.bounds_bits[1]),
                    0.0,
                ],
                [
                    f64::from_bits(mixed.bounds_bits[2]),
                    f64::from_bits(mixed.bounds_bits[3]),
                    height,
                ],
            ];
        }
        if let Some(boolean) = &self.boolean {
            if boolean.operation == BooleanOperation::Cut
                && self.pocket_depth_bits.is_none()
                && let Some(profile) = &boolean.profile
                && profile
                    .rounded_rectangle_side_overlap_area(width, depth)
                    .is_some()
            {
                let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
                let [min_x, min_y, max_x, max_y] = if min_y < 0.0 && max_y > depth {
                    if min_x > 0.0 {
                        [0.0, 0.0, min_x, depth]
                    } else {
                        [max_x, 0.0, width, depth]
                    }
                } else if min_y > 0.0 {
                    [0.0, 0.0, width, min_y]
                } else {
                    [0.0, max_y, width, depth]
                };
                return [[min_x, min_y, 0.0], [max_x, max_y, height]];
            }
            if matches!(
                boolean.operation,
                BooleanOperation::Union | BooleanOperation::Intersect
            ) && let Some(circle) = boolean.circle
            {
                if boolean.operation == BooleanOperation::Intersect
                    && let Some((_, [min_x, min_y, max_x, max_y])) = circle
                        .side_overlap(width, depth)
                        .or_else(|| circle.corner_overlap(width, depth))
                        .or_else(|| circle.outside_side_overlap(width, depth))
                        .or_else(|| circle.center_on_side_overlap(width, depth))
                        .or_else(|| circle.center_on_corner_overlap(width, depth))
                        .or_else(|| circle.outside_corner_overlap(width, depth))
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                let center_x = f64::from_bits(circle.center_x_bits);
                let center_y = f64::from_bits(circle.center_y_bits);
                let radius = f64::from_bits(circle.radius_bits);
                return if boolean.operation == BooleanOperation::Union {
                    [
                        [
                            (center_x - radius).min(0.0),
                            (center_y - radius).min(0.0),
                            0.0,
                        ],
                        [
                            (center_x + radius).max(width),
                            (center_y + radius).max(depth),
                            height,
                        ],
                    ]
                } else {
                    [
                        [center_x - radius, center_y - radius, 0.0],
                        [center_x + radius, center_y + radius, height],
                    ]
                };
            }
            if matches!(
                boolean.operation,
                BooleanOperation::Union | BooleanOperation::Intersect
            ) && let Some(profile) = &boolean.profile
            {
                let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
                if boolean.operation == BooleanOperation::Intersect
                    && let Some((_, [min_x, min_y, max_x, max_y])) =
                        profile.d_profile_arc_only_clipped_side_overlap(width, depth)
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                if boolean.operation == BooleanOperation::Intersect
                    && let Some((_, [min_x, min_y, max_x, max_y])) =
                        profile.capsule_side_overlap(width, depth)
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                if boolean.operation == BooleanOperation::Intersect
                    && let Some((_, [min_x, min_y, max_x, max_y])) =
                        profile.rounded_rectangle_chord_side_overlap(width, depth)
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                if boolean.operation == BooleanOperation::Intersect
                    && let Some((_, [min_x, min_y, max_x, max_y])) =
                        profile.strict_convex_line_clipped_side_overlap(width, depth)
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                if boolean.operation == BooleanOperation::Intersect
                    && let Some((_, [min_x, min_y, max_x, max_y])) =
                        profile.strict_convex_arc_only_clipped_side_overlap(width, depth)
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                if boolean.operation == BooleanOperation::Intersect
                    && let Some((_, [min_x, min_y, max_x, max_y])) =
                        profile.strict_convex_line_arc_clipped_side_overlap(width, depth)
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                if boolean.operation == BooleanOperation::Intersect
                    && let Some((_, [min_x, min_y, max_x, max_y])) = profile
                        .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
                        .or_else(|| {
                            profile.strict_convex_line_arc_clipped_south_west_corner_overlap(
                                width, depth,
                            )
                        })
                        .or_else(|| {
                            profile.strict_convex_line_arc_clipped_north_east_corner_overlap(
                                width, depth,
                            )
                        })
                        .or_else(|| {
                            profile.strict_convex_line_arc_clipped_north_west_corner_overlap(
                                width, depth,
                            )
                        })
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                if boolean.operation == BooleanOperation::Intersect
                    && let Some([min_x, min_y, max_x, max_y]) =
                        profile.rounded_rectangle_arc_clipped_corner_overlap_bounds(width, depth)
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                if boolean.operation == BooleanOperation::Intersect
                    && let Some([min_x, min_y, max_x, max_y]) = profile
                        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_bounds(width, depth)
                {
                    return [[min_x, min_y, 0.0], [max_x, max_y, height]];
                }
                return if boolean.operation == BooleanOperation::Union {
                    [
                        [min_x.min(0.0), min_y.min(0.0), 0.0],
                        [max_x.max(width), max_y.max(depth), height],
                    ]
                } else {
                    [
                        [min_x.max(0.0), min_y.max(0.0), 0.0],
                        [max_x.min(width), max_y.min(depth), height],
                    ]
                };
            }
            let tool = [
                f64::from_bits(boolean.min_x_bits),
                f64::from_bits(boolean.min_y_bits),
                f64::from_bits(boolean.min_x_bits) + f64::from_bits(boolean.width_bits),
                f64::from_bits(boolean.min_y_bits) + f64::from_bits(boolean.depth_bits),
            ];
            let boolean_bounds = match boolean.operation {
                BooleanOperation::Union => rectangular_union_bounds(width, depth, tool),
                BooleanOperation::Intersect => rectangular_intersection_bounds(width, depth, tool),
                BooleanOperation::Cut | BooleanOperation::Split => None,
            };
            if let Some([min_x, min_y, max_x, max_y]) = boolean_bounds {
                return [[min_x, min_y, 0.0], [max_x, max_y, height]];
            }
        }
        [[0.0; 3], [width, depth, height]]
    }

    #[must_use]
    pub fn producer_feature_id(&self) -> FeatureId {
        self.shell.as_ref().map_or_else(
            || {
                self.boolean
                    .as_ref()
                    .map_or(self.extrusion_feature_id, |cut| cut.feature_id)
            },
            |shell| {
                shell
                    .edge_finish_feature_id
                    .unwrap_or(shell.shell_feature_id)
            },
        )
    }

    #[must_use]
    pub fn evaluator(&self) -> &'static str {
        if let Some(shell) = &self.shell {
            return if shell.edge_finish_feature_id.is_some() {
                EXACT_BOX_FINISH_EVALUATOR_V1
            } else {
                EXACT_BOX_SHELL_EVALUATOR_V1
            };
        }
        match self.boolean.as_ref().map(|boolean| boolean.operation) {
            Some(BooleanOperation::Cut) if self.pocket_depth_bits.is_some() => {
                EXACT_POCKET_EVALUATOR_V1
            }
            Some(BooleanOperation::Cut)
                if self
                    .boolean
                    .as_ref()
                    .is_some_and(|boolean| boolean.circle.is_some()) =>
            {
                EXACT_CIRCULAR_CUT_EVALUATOR_V1
            }
            Some(BooleanOperation::Cut) => EXACT_THROUGH_CUT_EVALUATOR_V1,
            Some(BooleanOperation::Union) => EXACT_BOOLEAN_UNION_EVALUATOR_V1,
            Some(BooleanOperation::Intersect) => EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1,
            Some(BooleanOperation::Split) => EXACT_BOOLEAN_SPLIT_EVALUATOR_V1,
            None if self.circle.is_some() => EXACT_CIRCLE_EVALUATOR_V1,
            None if self
                .mixed_profile
                .as_ref()
                .is_some_and(|profile| profile.has_only_line_segments()) =>
            {
                EXACT_LINEAR_PROFILE_EVALUATOR_V1
            }
            None if self.mixed_profile.is_some() => EXACT_ARC_PROFILE_EVALUATOR_V1,
            None => EXACT_RECTANGLE_EVALUATOR_V1,
        }
    }

    #[must_use]
    pub fn profile_feature_id_for_role(&self, role: ExactFaceRole) -> Option<FeatureId> {
        match role {
            ExactFaceRole::Top
            | ExactFaceRole::Bottom
            | ExactFaceRole::East
            | ExactFaceRole::West => Some(self.profile_feature_id),
            ExactFaceRole::ArcSide => self
                .boolean
                .as_ref()
                .filter(|boolean| {
                    matches!(
                        boolean.operation,
                        BooleanOperation::Union
                            | BooleanOperation::Intersect
                            | BooleanOperation::Split
                    ) && boolean
                        .profile
                        .as_ref()
                        .is_some_and(|profile| !profile.has_only_line_segments())
                })
                .map_or(Some(self.profile_feature_id), |boolean| {
                    Some(boolean.profile_feature_id)
                }),
            ExactFaceRole::CircleSide => self
                .boolean
                .as_ref()
                .filter(|boolean| {
                    matches!(
                        boolean.operation,
                        BooleanOperation::Union
                            | BooleanOperation::Intersect
                            | BooleanOperation::Split
                    ) && boolean.circle.is_some()
                })
                .map_or(Some(self.profile_feature_id), |boolean| {
                    Some(boolean.profile_feature_id)
                }),
            ExactFaceRole::LinearSide => self
                .boolean
                .as_ref()
                .filter(|boolean| {
                    matches!(
                        boolean.operation,
                        BooleanOperation::Union
                            | BooleanOperation::Intersect
                            | BooleanOperation::Split
                    )
                })
                .map_or(Some(self.profile_feature_id), |boolean| {
                    Some(boolean.profile_feature_id)
                }),
            ExactFaceRole::CutCircle | ExactFaceRole::CutLinear | ExactFaceRole::CutArc => {
                self.boolean.as_ref().map(|cut| cut.profile_feature_id)
            }
            ExactFaceRole::CutWest
            | ExactFaceRole::CutEast
            | ExactFaceRole::CutSouth
            | ExactFaceRole::CutNorth
            | ExactFaceRole::PocketFloor
            | ExactFaceRole::PocketWest
            | ExactFaceRole::PocketEast
            | ExactFaceRole::PocketSouth
            | ExactFaceRole::PocketNorth => self.boolean.as_ref().map(|cut| cut.profile_feature_id),
            ExactFaceRole::RevolveBottom
            | ExactFaceRole::RevolveBody
            | ExactFaceRole::RevolveShoulder
            | ExactFaceRole::RevolveNeck
            | ExactFaceRole::RevolveMouth
            | ExactFaceRole::RevolveSide0
            | ExactFaceRole::RevolveSide1
            | ExactFaceRole::RevolveStart
            | ExactFaceRole::RevolveEnd
            | ExactFaceRole::ShellOuterBottom
            | ExactFaceRole::ShellOuterBody
            | ExactFaceRole::ShellOuterShoulder
            | ExactFaceRole::ShellOuterNeck
            | ExactFaceRole::ShellRim
            | ExactFaceRole::ShellInnerBottom
            | ExactFaceRole::ShellInnerBody
            | ExactFaceRole::ShellInnerShoulder
            | ExactFaceRole::ShellInnerNeck
            | ExactFaceRole::PlanarOffsetFace
            | ExactFaceRole::SweepStart
            | ExactFaceRole::SweepEnd
            | ExactFaceRole::SweepSide0
            | ExactFaceRole::SweepSide1
            | ExactFaceRole::SweepSide2
            | ExactFaceRole::SweepSide3
            | ExactFaceRole::LoftStart
            | ExactFaceRole::LoftEnd
            | ExactFaceRole::LoftSide => None,
            ExactFaceRole::BoxShellOuterBottom
            | ExactFaceRole::BoxShellOuterEast
            | ExactFaceRole::BoxShellRim => Some(self.profile_feature_id),
        }
    }

    pub(crate) fn expected_face_roles(&self) -> &'static [ExactFaceRole] {
        if self.shell.is_some() {
            return &BOX_SHELL_FACE_ROLES;
        }
        if self.boolean.as_ref().is_some_and(|boolean| {
            boolean.operation == BooleanOperation::Cut && boolean.circle.is_some()
        }) {
            let [width, depth, _] = self.dimensions_mm();
            let side_overlap = self.boolean.as_ref().is_some_and(|boolean| {
                boolean.circle.is_some_and(|circle| {
                    circle.side_overlap(width, depth).is_some()
                        || circle.corner_overlap(width, depth).is_some()
                        || circle.outside_side_overlap(width, depth).is_some()
                        || circle.center_on_side_overlap(width, depth).is_some()
                        || circle.center_on_corner_overlap(width, depth).is_some()
                        || circle.outside_corner_overlap(width, depth).is_some()
                })
            });
            let east_overlap = side_overlap
                && self.boolean.as_ref().is_some_and(|boolean| {
                    boolean.circle.is_some_and(|circle| {
                        f64::from_bits(circle.center_x_bits) + f64::from_bits(circle.radius_bits)
                            > width + 1.0e-6
                    })
                });
            if self.pocket_depth_bits.is_some() && side_overlap {
                if east_overlap {
                    &WEST_CIRCULAR_POCKET_FACE_ROLES
                } else {
                    &CIRCULAR_POCKET_FACE_ROLES
                }
            } else if east_overlap {
                &WEST_CIRCULAR_CUT_FACE_ROLES
            } else {
                &CIRCULAR_CUT_FACE_ROLES
            }
        } else if self.boolean.as_ref().is_some_and(|boolean| {
            matches!(
                boolean.operation,
                BooleanOperation::Union | BooleanOperation::Intersect
            ) && boolean.circle.is_some()
        }) {
            &CIRCLE_EXTRUSION_FACE_ROLES
        } else if self.boolean.as_ref().is_some_and(|boolean| {
            boolean.operation == BooleanOperation::Split && boolean.circle.is_some()
        }) {
            let [width, depth, _] = self.dimensions_mm();
            if self.boolean.as_ref().is_some_and(|boolean| {
                boolean.circle.is_some_and(|circle| {
                    circle.side_overlap(width, depth).is_some()
                        || circle.corner_overlap(width, depth).is_some()
                        || circle.outside_side_overlap(width, depth).is_some()
                        || circle.center_on_side_overlap(width, depth).is_some()
                        || circle.center_on_corner_overlap(width, depth).is_some()
                        || circle.outside_corner_overlap(width, depth).is_some()
                })
            }) {
                &CIRCLE_EXTRUSION_FACE_ROLES
            } else {
                &EXTRUSION_FACE_ROLES
            }
        } else if let Some(boolean) = self
            .boolean
            .as_ref()
            .filter(|boolean| boolean.profile.is_some())
        {
            if boolean.operation == BooleanOperation::Split {
                let dimensions = self.dimensions_mm();
                if boolean.profile.as_ref().is_some_and(|profile| {
                    profile
                        .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                        .is_some()
                }) {
                    &LINEAR_EXTRUSION_FACE_ROLES
                } else if boolean.profile.as_ref().is_some_and(|profile| {
                    profile.is_line_arc_d_profile()
                        || profile.is_line_arc_capsule_profile()
                        || profile.is_line_arc_rounded_rectangle_profile()
                        || profile.is_strict_convex_line_arc_profile()
                }) {
                    &ARC_EXTRUSION_FACE_ROLES
                } else {
                    &EXTRUSION_FACE_ROLES
                }
            } else if matches!(
                boolean.operation,
                BooleanOperation::Union | BooleanOperation::Intersect
            ) {
                let dimensions = self.dimensions_mm();
                if boolean.profile.as_ref().is_some_and(|profile| {
                    profile.has_only_line_segments()
                        || boolean.operation == BooleanOperation::Union
                            && profile
                                .strict_convex_line_clipped_side_overlap(
                                    dimensions[0],
                                    dimensions[1],
                                )
                                .is_some()
                        || boolean.operation == BooleanOperation::Intersect
                            && profile
                                .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                                .is_some()
                }) {
                    &LINEAR_EXTRUSION_FACE_ROLES
                } else {
                    &ARC_EXTRUSION_FACE_ROLES
                }
            } else if boolean.operation == BooleanOperation::Cut
                && boolean.profile.as_ref().is_some_and(|profile| {
                    let dimensions = self.dimensions_mm();
                    profile
                        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                            dimensions[0],
                            dimensions[1],
                        )
                        .is_some()
                })
            {
                if self.pocket_depth_bits.is_some() {
                    &ARC_POLYGON_POCKET_FACE_ROLES
                } else {
                    &ARC_POLYGON_CUT_FACE_ROLES
                }
            } else if self.pocket_depth_bits.is_some() {
                &POLYGON_POCKET_FACE_ROLES
            } else if boolean.profile.as_ref().is_some_and(|profile| {
                let dimensions = self.dimensions_mm();
                profile
                    .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                    .is_some()
                    && f64::from_bits(profile.bounds_bits[0]) > 0.0
            }) {
                &WEST_POLYGON_CUT_FACE_ROLES
            } else {
                &POLYGON_CUT_FACE_ROLES
            }
        } else if self.pocket_depth_bits.is_some() {
            &POCKET_FACE_ROLES
        } else if self
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.operation == BooleanOperation::Cut)
        {
            &THROUGH_CUT_FACE_ROLES
        } else if self.circle.is_some() {
            &CIRCLE_EXTRUSION_FACE_ROLES
        } else if self
            .mixed_profile
            .as_ref()
            .is_some_and(ExactMixedProfile::has_only_line_segments)
        {
            &LINEAR_EXTRUSION_FACE_ROLES
        } else if self.mixed_profile.is_some() {
            &ARC_EXTRUSION_FACE_ROLES
        } else {
            &EXTRUSION_FACE_ROLES
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactProductError {
    DefinitionNotFound(DefinitionId),
    ProfileNotFound(FeatureId),
    UnsupportedDefinition,
    UnsupportedProfile,
    UnsupportedExtrusion,
    UnsupportedThroughCut,
    UnsupportedBoolean(BooleanOperation),
    UnsupportedShell,
    EmptyModelExport,
    InvalidMeshExport,
    InvalidWorkerEvidence,
    StaleResult,
    BodyOutputNotFound {
        definition_id: DefinitionId,
        body_id: BodyId,
    },
    NonTerminalBodyResult {
        definition_id: DefinitionId,
        body_id: BodyId,
        producer_feature_id: FeatureId,
    },
    ConflictingBodyTerminals {
        definition_id: DefinitionId,
        body_id: BodyId,
    },
    ConflictingBodyPublication {
        definition_id: DefinitionId,
        body_id: BodyId,
    },
    DuplicateResult {
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
    },
    DuplicateDerivedResult {
        piece: DerivedIdentity,
    },
}

impl fmt::Display for ExactProductError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} was not found", id.0),
            Self::ProfileNotFound(id) => write!(formatter, "profile {} was not found", id.0),
            Self::UnsupportedDefinition => {
                formatter.write_str("exact M3 supports exactly one rectangle extrusion")
            }
            Self::UnsupportedProfile => {
                formatter.write_str("exact M3 supports an origin-based axis-aligned rectangle")
            }
            Self::UnsupportedExtrusion => {
                formatter.write_str("exact M3 supports a finite positive extrusion")
            }
            Self::UnsupportedThroughCut => formatter.write_str(
                "exact M3 supports one strictly bounded axis-aligned rectangle through-cut",
            ),
            Self::UnsupportedBoolean(operation) => write!(
                formatter,
                "exact feature-chain evaluator does not support {operation:?} in this envelope"
            ),
            Self::UnsupportedShell => formatter
                .write_str("exact M6 shell thickness is outside the conservative bottle envelope"),
            Self::EmptyModelExport => formatter.write_str("the visible exact model is empty"),
            Self::InvalidMeshExport => {
                formatter.write_str("the accepted exact tessellation contains an invalid facet")
            }
            Self::InvalidWorkerEvidence => {
                formatter.write_str("exact worker evidence does not match the canonical request")
            }
            Self::StaleResult => formatter.write_str("exact result is stale for the snapshot"),
            Self::BodyOutputNotFound {
                definition_id,
                body_id,
            } => write!(
                formatter,
                "body {} in definition {} has no exact terminal output",
                body_id.0, definition_id.0
            ),
            Self::NonTerminalBodyResult {
                definition_id,
                body_id,
                producer_feature_id,
            } => write!(
                formatter,
                "feature {} is not the terminal output of body {} in definition {}",
                producer_feature_id.0, body_id.0, definition_id.0
            ),
            Self::ConflictingBodyTerminals {
                definition_id,
                body_id,
            } => write!(
                formatter,
                "body {} in definition {} has conflicting terminal outputs",
                body_id.0, definition_id.0
            ),
            Self::ConflictingBodyPublication {
                definition_id,
                body_id,
            } => write!(
                formatter,
                "body {} in definition {} has conflicting exact publications",
                body_id.0, definition_id.0
            ),
            Self::DuplicateResult {
                definition_id,
                producer_feature_id,
            } => write!(
                formatter,
                "duplicate exact result for definition {} producer {}",
                definition_id.0, producer_feature_id.0
            ),
            Self::DuplicateDerivedResult { piece } => write!(
                formatter,
                "duplicate exact result for derived rule {} slot path",
                piece.root_rule_node_id.0
            ),
        }
    }
}

impl std::error::Error for ExactProductError {}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanarOffsetWorkerEvidence {
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub backend: String,
    pub tolerance: String,
    pub bounds_mm: [[f64; 3]; 2],
    pub area_mm2: f64,
    pub face_ordinal: u32,
    pub lineage_digest: String,
    pub corroborating_geometry_fingerprint: String,
}

pub fn build_planar_offset_package(
    request: &ExactPlanarOffsetRequest,
    evidence: PlanarOffsetWorkerEvidence,
) -> Result<ExactPlanarOffsetPackage, ExactProductError> {
    let PlanarOffsetWorkerEvidence {
        exact_input_digest,
        result_fingerprint,
        backend,
        tolerance,
        bounds_mm: worker_bounds_mm,
        area_mm2: worker_area_mm2,
        face_ordinal,
        lineage_digest,
        corroborating_geometry_fingerprint,
    } = evidence;
    let expected_bounds = request.expected_bounds_mm();
    let expected_area_mm2 = request.expected_area_mm2();
    let expected_lineage = canonical_reference_lineage_digest(
        request.document_id,
        request.offset_feature_id,
        ExactFaceRole::PlanarOffsetFace.semantic_role(),
        ExactFaceRole::PlanarOffsetFace.source_element_id(),
        ExactFaceRole::PlanarOffsetFace.expected_type(),
    );
    if expected_bounds
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
        || !expected_area_mm2.is_finite()
        || worker_bounds_mm
            .iter()
            .flatten()
            .zip(expected_bounds.iter().flatten())
            .any(|(actual, expected)| !actual.is_finite() || (actual - expected).abs() > 1.0e-6)
        || !worker_area_mm2.is_finite()
        || (worker_area_mm2 - expected_area_mm2).abs() > 1.0e-6
        || face_ordinal != 0
        || lineage_digest != expected_lineage
        || exact_input_digest.is_empty()
        || result_fingerprint.is_empty()
        || backend.is_empty()
        || tolerance.is_empty()
        || corroborating_geometry_fingerprint.is_empty()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let [min, max] = expected_bounds;
    let vertices = vec![
        ExactVertex {
            position_mm: [min[0], min[1], 0.0],
        },
        ExactVertex {
            position_mm: [max[0], min[1], 0.0],
        },
        ExactVertex {
            position_mm: [max[0], max[1], 0.0],
        },
        ExactVertex {
            position_mm: [min[0], max[1], 0.0],
        },
    ];
    let triangles = vec![
        ExactTriangle {
            vertex_indices: [0, 1, 2],
            face_role: Some(ExactFaceRole::PlanarOffsetFace),
        },
        ExactTriangle {
            vertex_indices: [0, 2, 3],
            face_role: Some(ExactFaceRole::PlanarOffsetFace),
        },
    ];
    let reference = BodySubshapeRef {
        schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
        document_id: request.document_id,
        definition_id: request.definition_id,
        profile_feature_id: request.profile_feature_id,
        producer_feature_id: request.offset_feature_id,
        semantic_role: ExactFaceRole::PlanarOffsetFace.semantic_role().to_owned(),
        source_element_id: ExactFaceRole::PlanarOffsetFace
            .source_element_id()
            .to_owned(),
        expected_type: ExactFaceRole::PlanarOffsetFace.expected_type().to_owned(),
        expected_cardinality: 1,
        stability: ReferenceStability::Guaranteed,
        canonical_input_digest: request.canonical_input_digest.clone(),
        exact_input_digest: exact_input_digest.clone(),
        result_fingerprint: result_fingerprint.clone(),
        evaluator: request.evaluator().to_owned(),
        backend: backend.clone(),
        tolerance: tolerance.clone(),
        lineage_digest,
        corroborating_geometry_fingerprint,
    };
    let package = ExactPlanarOffsetPackage {
        identity: ExactPlanarOffsetIdentity {
            schema: EXACT_PLANAR_OFFSET_SCHEMA_V1.to_owned(),
            document_id: request.document_id,
            source_revision: request.source_revision,
            source_digest: request.source_digest.clone(),
            definition_id: request.definition_id,
            profile_feature_id: request.profile_feature_id,
            offset_feature_id: request.offset_feature_id,
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest,
            result_fingerprint,
            evaluator: request.evaluator().to_owned(),
            backend,
            tolerance,
        },
        bounds_mm: expected_bounds,
        area_mm2: worker_area_mm2,
        vertices,
        triangles,
        reference,
    };
    package.validate_for_request(request)?;
    Ok(package)
}

#[derive(Clone, Debug, PartialEq)]
pub struct SweepWorkerFaceEvidence {
    pub role: ExactFaceRole,
    pub face_ordinal: u32,
    pub lineage_digest: String,
    pub corroborating_geometry_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SweepWorkerEvidence {
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub backend: String,
    pub tolerance: String,
    pub bounds_mm: [[f64; 3]; 2],
    pub volume_mm3: f64,
    pub faces: Vec<SweepWorkerFaceEvidence>,
}

pub fn build_sweep_package(
    request: &ExactSweepRequest,
    evidence: SweepWorkerEvidence,
) -> Result<ExactSweepPackage, ExactProductError> {
    let expected_roles = [
        ExactFaceRole::SweepStart,
        ExactFaceRole::SweepEnd,
        ExactFaceRole::SweepSide0,
        ExactFaceRole::SweepSide1,
        ExactFaceRole::SweepSide2,
        ExactFaceRole::SweepSide3,
    ];
    let [worker_min, worker_max] = evidence.bounds_mm;
    let [expected_min, expected_max] = request.expected_bounds_mm();
    if worker_min
        .into_iter()
        .chain(worker_max)
        .zip(expected_min.into_iter().chain(expected_max))
        .any(|(actual, expected)| !actual.is_finite() || (actual - expected).abs() > 1.0e-6)
        || !evidence.volume_mm3.is_finite()
        || (evidence.volume_mm3 - request.expected_volume_mm3()).abs() > 1.0e-6
        || evidence.exact_input_digest.is_empty()
        || evidence.result_fingerprint.is_empty()
        || evidence.backend.is_empty()
        || evidence.tolerance.is_empty()
        || evidence.faces.len() != expected_roles.len()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut ordinals = Vec::with_capacity(expected_roles.len());
    let mut references = Vec::with_capacity(expected_roles.len());
    for role in expected_roles {
        let matching = evidence
            .faces
            .iter()
            .filter(|face| face.role == role)
            .collect::<Vec<_>>();
        let [face] = matching.as_slice() else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        let expected_lineage = canonical_reference_lineage_digest(
            request.document_id,
            request.sweep_feature_id,
            role.semantic_role(),
            role.source_element_id(),
            role.expected_type(),
        );
        if face.face_ordinal >= 6
            || face.lineage_digest != expected_lineage
            || face.corroborating_geometry_fingerprint.is_empty()
            || ordinals.contains(&face.face_ordinal)
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        ordinals.push(face.face_ordinal);
        references.push(BodySubshapeRef {
            schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
            document_id: request.document_id,
            definition_id: request.definition_id,
            profile_feature_id: request.profile_feature_id,
            producer_feature_id: request.sweep_feature_id,
            semantic_role: role.semantic_role().to_owned(),
            source_element_id: role.source_element_id().to_owned(),
            expected_type: role.expected_type().to_owned(),
            expected_cardinality: 1,
            stability: ReferenceStability::Guaranteed,
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest: evidence.exact_input_digest.clone(),
            result_fingerprint: evidence.result_fingerprint.clone(),
            evaluator: request.evaluator().to_owned(),
            backend: evidence.backend.clone(),
            tolerance: evidence.tolerance.clone(),
            lineage_digest: face.lineage_digest.clone(),
            corroborating_geometry_fingerprint: face.corroborating_geometry_fingerprint.clone(),
        });
    }
    let vertices = request
        .expected_vertices_mm()
        .into_iter()
        .map(|position_mm| ExactVertex { position_mm })
        .collect();
    let triangles = [
        ([0, 2, 1], ExactFaceRole::SweepStart),
        ([0, 3, 2], ExactFaceRole::SweepStart),
        ([4, 5, 6], ExactFaceRole::SweepEnd),
        ([4, 6, 7], ExactFaceRole::SweepEnd),
        ([0, 1, 5], ExactFaceRole::SweepSide0),
        ([0, 5, 4], ExactFaceRole::SweepSide0),
        ([1, 2, 6], ExactFaceRole::SweepSide1),
        ([1, 6, 5], ExactFaceRole::SweepSide1),
        ([2, 3, 7], ExactFaceRole::SweepSide2),
        ([2, 7, 6], ExactFaceRole::SweepSide2),
        ([3, 0, 4], ExactFaceRole::SweepSide3),
        ([3, 4, 7], ExactFaceRole::SweepSide3),
    ]
    .into_iter()
    .map(|(vertex_indices, role)| ExactTriangle {
        vertex_indices,
        face_role: Some(role),
    })
    .collect();
    let package = ExactSweepPackage {
        identity: ExactSweepIdentity {
            schema: EXACT_SWEEP_SCHEMA_V1.to_owned(),
            document_id: request.document_id,
            source_revision: request.source_revision,
            source_digest: request.source_digest.clone(),
            definition_id: request.definition_id,
            profile_feature_id: request.profile_feature_id,
            path_feature_id: request.path_feature_id,
            sweep_feature_id: request.sweep_feature_id,
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest: evidence.exact_input_digest,
            result_fingerprint: evidence.result_fingerprint,
            evaluator: request.evaluator().to_owned(),
            backend: evidence.backend,
            tolerance: evidence.tolerance,
        },
        bounds_mm: request.expected_bounds_mm(),
        volume_mm3: evidence.volume_mm3,
        vertices,
        triangles,
        references,
    };
    package.validate_for_request(request)?;
    Ok(package)
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoftWorkerEvidence {
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub backend: String,
    pub tolerance: String,
    pub bounds_mm: [[f64; 3]; 2],
    pub volume_mm3: f64,
    pub topology_counts: [u32; 5],
    pub faces: Vec<SweepWorkerFaceEvidence>,
}

pub fn build_loft_package(
    request: &ExactLoftRequest,
    evidence: LoftWorkerEvidence,
) -> Result<ExactLoftPackage, ExactProductError> {
    let roles = [
        ExactFaceRole::LoftStart,
        ExactFaceRole::LoftEnd,
        ExactFaceRole::LoftSide,
    ];
    if evidence.exact_input_digest.is_empty()
        || evidence.result_fingerprint.is_empty()
        || evidence.backend.is_empty()
        || evidence.tolerance.is_empty()
        || evidence.faces.len() != roles.len()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut references = Vec::with_capacity(roles.len());
    let mut ordinals = Vec::with_capacity(roles.len());
    for role in roles {
        let matching = evidence
            .faces
            .iter()
            .filter(|face| face.role == role)
            .collect::<Vec<_>>();
        let [face] = matching.as_slice() else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        let profile_feature_id = match role {
            ExactFaceRole::LoftEnd => request.sections.last().unwrap().profile_feature_id,
            _ => request.sections.first().unwrap().profile_feature_id,
        };
        let expected_lineage = canonical_reference_lineage_digest(
            request.document_id,
            request.loft_feature_id,
            role.semantic_role(),
            role.source_element_id(),
            role.expected_type(),
        );
        if face.face_ordinal >= evidence.topology_counts[2]
            || ordinals.contains(&face.face_ordinal)
            || face.lineage_digest != expected_lineage
            || face.corroborating_geometry_fingerprint.is_empty()
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        ordinals.push(face.face_ordinal);
        references.push(BodySubshapeRef {
            schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
            document_id: request.document_id,
            definition_id: request.definition_id,
            profile_feature_id,
            producer_feature_id: request.loft_feature_id,
            semantic_role: role.semantic_role().to_owned(),
            source_element_id: role.source_element_id().to_owned(),
            expected_type: role.expected_type().to_owned(),
            expected_cardinality: 1,
            stability: ReferenceStability::Guaranteed,
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest: evidence.exact_input_digest.clone(),
            result_fingerprint: evidence.result_fingerprint.clone(),
            evaluator: request.evaluator().to_owned(),
            backend: evidence.backend.clone(),
            tolerance: evidence.tolerance.clone(),
            lineage_digest: face.lineage_digest.clone(),
            corroborating_geometry_fingerprint: face.corroborating_geometry_fingerprint.clone(),
        });
    }
    let package = ExactLoftPackage {
        identity: ExactLoftIdentity {
            schema: EXACT_LOFT_SCHEMA_V1.to_owned(),
            document_id: request.document_id,
            source_revision: request.source_revision,
            source_digest: request.source_digest.clone(),
            definition_id: request.definition_id,
            loft_feature_id: request.loft_feature_id,
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest: evidence.exact_input_digest,
            result_fingerprint: evidence.result_fingerprint,
            evaluator: request.evaluator().to_owned(),
            backend: evidence.backend,
            tolerance: evidence.tolerance,
        },
        bounds_mm: evidence.bounds_mm,
        volume_mm3: evidence.volume_mm3,
        topology_counts: evidence.topology_counts,
        references,
    };
    package.validate_for_request(request)?;
    Ok(package)
}

fn transform_workplane_point(frame: [f64; 12], point: [f64; 3]) -> [f64; 3] {
    [
        frame[0] + frame[3] * point[0] + frame[6] * point[1] + frame[9] * point[2],
        frame[1] + frame[4] * point[0] + frame[7] * point[1] + frame[10] * point[2],
        frame[2] + frame[5] * point[0] + frame[8] * point[1] + frame[11] * point[2],
    ]
}

pub fn build_box_render_package<const N: usize>(
    request: &ExactFeatureChainRequest,
    exact_input_digest: String,
    result_fingerprint: String,
    backend: String,
    tolerance: String,
    worker_bounds_mm: [[f64; 3]; 2],
    face_evidence: [(ExactFaceRole, String, String); N],
) -> Result<ExactRenderPackage, ExactProductError> {
    let [worker_min, worker_max] = worker_bounds_mm;
    let [expected_min, expected_max] = request.expected_bounds_mm();
    if worker_min
        .into_iter()
        .chain(worker_max)
        .any(|value| !value.is_finite())
        || (0..3).any(|axis| {
            (worker_min[axis] - expected_min[axis]).abs() > 1.0e-6
                || (worker_max[axis] - expected_max[axis]).abs() > 1.0e-6
        })
        || exact_input_digest.is_empty()
        || result_fingerprint.is_empty()
        || backend.is_empty()
        || tolerance.is_empty()
        || !valid_evidence_roles(request, &face_evidence)
        || face_evidence
            .iter()
            .any(|(_, lineage_digest, fingerprint)| {
                lineage_digest.is_empty() || fingerprint.is_empty()
            })
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let (mut vertices, mut triangles) = render_mesh(request)?;
    let (min, max) = if let Some(frame_bits) = request.workplane_frame_bits {
        let frame = frame_bits.map(f64::from_bits);
        let determinant = frame[3] * (frame[7] * frame[11] - frame[8] * frame[10])
            - frame[6] * (frame[4] * frame[11] - frame[5] * frame[10])
            + frame[9] * (frame[4] * frame[8] - frame[5] * frame[7]);
        if determinant < 0.0 {
            for triangle in &mut triangles {
                triangle.vertex_indices.swap(1, 2);
            }
        }
        for vertex in &mut vertices {
            vertex.position_mm = transform_workplane_point(frame, vertex.position_mm);
        }
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for vertex in &vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position_mm[axis]);
                max[axis] = max[axis].max(vertex.position_mm[axis]);
            }
        }
        (min, max)
    } else {
        (expected_min, expected_max)
    };
    let references = face_evidence
        .into_iter()
        .map(
            |(role, lineage_digest, corroborating_geometry_fingerprint)| {
                Ok(BodySubshapeRef {
                    schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
                    document_id: request.document_id,
                    definition_id: request.definition_id,
                    profile_feature_id: request
                        .profile_feature_id_for_role(role)
                        .ok_or(ExactProductError::InvalidWorkerEvidence)?,
                    producer_feature_id: request.producer_feature_id(),
                    semantic_role: role.semantic_role().to_owned(),
                    source_element_id: role.source_element_id().to_owned(),
                    expected_type: role.expected_type().to_owned(),
                    expected_cardinality: 1,
                    stability: ReferenceStability::Guaranteed,
                    canonical_input_digest: request.canonical_input_digest.clone(),
                    exact_input_digest: exact_input_digest.clone(),
                    result_fingerprint: result_fingerprint.clone(),
                    evaluator: request.evaluator().to_owned(),
                    backend: backend.clone(),
                    tolerance: tolerance.clone(),
                    lineage_digest,
                    corroborating_geometry_fingerprint,
                })
            },
        )
        .collect::<Result<Vec<_>, ExactProductError>>()?;
    if references
        .iter()
        .any(|reference| !reference.matches_request(request))
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    Ok(ExactRenderPackage {
        identity: BodyResultIdentity {
            schema: EXACT_PRODUCT_SCHEMA_V1.to_owned(),
            document_id: request.document_id,
            source_revision: request.source_revision,
            source_digest: request.source_digest.clone(),
            definition_id: request.definition_id,
            profile_feature_id: request.profile_feature_id,
            extrusion_feature_id: request.extrusion_feature_id,
            producer_feature_id: request.producer_feature_id(),
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest,
            result_fingerprint,
            evaluator: request.evaluator().to_owned(),
            backend,
            tolerance,
        },
        bounds_mm: [min, max],
        vertices,
        triangles,
        references,
    })
}

fn valid_evidence_roles<const N: usize>(
    request: &ExactFeatureChainRequest,
    evidence: &[(ExactFaceRole, String, String); N],
) -> bool {
    let mut roles = evidence
        .iter()
        .map(|(role, _, _)| *role)
        .collect::<Vec<_>>();
    roles.sort_unstable();
    if roles.windows(2).any(|pair| pair[0] == pair[1])
        || roles
            .iter()
            .any(|role| !request.expected_face_roles().contains(role))
        || evidence.iter().any(|(role, lineage, _)| {
            *lineage
                != canonical_reference_lineage_digest(
                    request.document_id,
                    request.producer_feature_id(),
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
                )
        })
    {
        return false;
    }
    let expected_roles = request.expected_face_roles();
    roles.len() == expected_roles.len() && roles == expected_roles.to_vec()
}

fn render_mesh(
    request: &ExactFeatureChainRequest,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let [width, depth, height] = request.dimensions_mm();
    let side_overlap_intersection = request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Intersect
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .rounded_rectangle_side_overlap_area(width, depth)
                    .is_some()
            })
    });
    let side_overlap_cut = request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Cut
            && request.pocket_depth_bits.is_none()
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .rounded_rectangle_side_overlap_area(width, depth)
                    .is_some()
            })
    });
    let corner_overlap_cut = request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Cut
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile.capsule_corner_overlap(width, depth).is_some()
                    || profile
                        .rounded_rectangle_corner_overlap_area(width, depth)
                        .is_some()
                    || profile
                        .rounded_rectangle_arc_clipped_corner_overlap_area(width, depth)
                        .is_some()
                    || profile
                        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(width, depth)
                        .is_some()
            })
    });
    let side_overlap_split = request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Split
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .rounded_rectangle_side_overlap_area(width, depth)
                    .is_some()
            })
    });
    let side_clipped_cut = request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Cut
            && !side_overlap_cut
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .d_profile_arc_only_clipped_side_overlap(width, depth)
                    .is_some()
                    || profile.capsule_side_overlap(width, depth).is_some()
                    || profile
                        .rounded_rectangle_chord_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_arc_only_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_south_west_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_north_east_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_north_west_corner_overlap(width, depth)
                        .is_some()
                    || request.pocket_depth_bits.is_some()
                        && profile
                            .rounded_rectangle_side_overlap_area(width, depth)
                            .is_some()
            })
    });
    let side_clipped_union = request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Union
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .d_profile_arc_only_clipped_side_overlap(width, depth)
                    .is_some()
                    || profile.capsule_side_overlap(width, depth).is_some()
                    || profile.capsule_corner_overlap(width, depth).is_some()
                    || profile
                        .rounded_rectangle_chord_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_arc_only_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_south_west_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_north_east_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_north_west_corner_overlap(width, depth)
                        .is_some()
            })
    });
    let curved_corner_overlap_split = request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Split
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .d_profile_arc_only_clipped_side_overlap(width, depth)
                    .is_some()
                    || profile.capsule_side_overlap(width, depth).is_some()
                    || profile.capsule_corner_overlap(width, depth).is_some()
                    || profile
                        .rounded_rectangle_chord_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_arc_only_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_south_west_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_north_east_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_north_west_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .rounded_rectangle_corner_overlap_area(width, depth)
                        .is_some()
                    || profile
                        .rounded_rectangle_arc_clipped_corner_overlap_area(width, depth)
                        .is_some()
                    || profile
                        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(width, depth)
                        .is_some()
            })
    });
    let curved_corner_overlap_intersection = request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Intersect
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .d_profile_arc_only_clipped_side_overlap(width, depth)
                    .is_some()
                    || profile.capsule_side_overlap(width, depth).is_some()
                    || profile.capsule_corner_overlap(width, depth).is_some()
                    || profile
                        .rounded_rectangle_chord_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_arc_only_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_side_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_south_west_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_north_east_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .strict_convex_line_arc_clipped_north_west_corner_overlap(width, depth)
                        .is_some()
                    || profile
                        .rounded_rectangle_corner_overlap_area(width, depth)
                        .is_some()
                    || profile
                        .rounded_rectangle_arc_clipped_corner_overlap_area(width, depth)
                        .is_some()
                    || profile
                        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(width, depth)
                        .is_some()
            })
    });
    if let Some(circle) = request.circle {
        return render_circle_mesh(circle, height);
    }
    if let Some(mixed) = &request.mixed_profile {
        return render_mixed_profile_mesh(mixed, height);
    }
    if side_overlap_split
        && let Some(profile) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
    {
        let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
        let tolerance = 1.0e-6;
        let (boxes, interface_faces) = if min_y + tolerance < 0.0
            && max_y - tolerance > depth
            && min_x > tolerance
            && min_x < width - tolerance
            && max_x > width + tolerance
        {
            (
                [[0.0, 0.0, min_x, depth], [min_x, 0.0, width, depth]],
                [0_usize, 1_usize],
            )
        } else if min_y + tolerance < 0.0
            && max_y - tolerance > depth
            && min_x < -tolerance
            && max_x > tolerance
            && max_x < width - tolerance
        {
            (
                [[0.0, 0.0, max_x, depth], [max_x, 0.0, width, depth]],
                [0_usize, 1_usize],
            )
        } else if min_x + tolerance < 0.0
            && max_x - tolerance > width
            && min_y > tolerance
            && min_y < depth - tolerance
            && max_y > depth + tolerance
        {
            (
                [[0.0, 0.0, width, min_y], [0.0, min_y, width, depth]],
                [2_usize, 3_usize],
            )
        } else if min_x + tolerance < 0.0
            && max_x - tolerance > width
            && min_y < -tolerance
            && max_y > tolerance
            && max_y < depth - tolerance
        {
            (
                [[0.0, 0.0, width, max_y], [0.0, max_y, width, depth]],
                [2_usize, 3_usize],
            )
        } else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        let mut vertices = Vec::with_capacity(16);
        let mut triangles = Vec::with_capacity(24);
        for ([box_min_x, box_min_y, box_max_x, box_max_y], interface_face) in
            boxes.into_iter().zip(interface_faces)
        {
            let offset = vertices.len() as u32;
            vertices.extend(
                [
                    [box_min_x, box_min_y, 0.0],
                    [box_max_x, box_min_y, 0.0],
                    [box_max_x, box_max_y, 0.0],
                    [box_min_x, box_max_y, 0.0],
                    [box_min_x, box_min_y, height],
                    [box_max_x, box_min_y, height],
                    [box_max_x, box_max_y, height],
                    [box_min_x, box_max_y, height],
                ]
                .map(|position_mm| ExactVertex { position_mm }),
            );
            for (indices, role) in [
                ([0, 2, 1], Some(ExactFaceRole::Bottom)),
                ([0, 3, 2], Some(ExactFaceRole::Bottom)),
                ([4, 5, 6], Some(ExactFaceRole::Top)),
                ([4, 6, 7], Some(ExactFaceRole::Top)),
                (
                    [1, 2, 6],
                    (interface_face == 0).then_some(ExactFaceRole::LinearSide),
                ),
                (
                    [1, 6, 5],
                    (interface_face == 0).then_some(ExactFaceRole::LinearSide),
                ),
                (
                    [0, 4, 7],
                    (interface_face == 1).then_some(ExactFaceRole::LinearSide),
                ),
                (
                    [0, 7, 3],
                    (interface_face == 1).then_some(ExactFaceRole::LinearSide),
                ),
                (
                    [3, 7, 6],
                    (interface_face == 2).then_some(ExactFaceRole::LinearSide),
                ),
                (
                    [3, 6, 2],
                    (interface_face == 2).then_some(ExactFaceRole::LinearSide),
                ),
                (
                    [0, 1, 5],
                    (interface_face == 3).then_some(ExactFaceRole::LinearSide),
                ),
                (
                    [0, 5, 4],
                    (interface_face == 3).then_some(ExactFaceRole::LinearSide),
                ),
            ] {
                triangles.push(ExactTriangle {
                    vertex_indices: indices.map(|index| index + offset),
                    face_role: role,
                });
            }
        }
        return Ok((vertices, triangles));
    }
    if curved_corner_overlap_split
        && let Some(profile) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
    {
        if profile
            .d_profile_arc_only_clipped_side_overlap(width, depth)
            .is_some()
            || profile.capsule_side_overlap(width, depth).is_some()
            || profile
                .rounded_rectangle_chord_side_overlap(width, depth)
                .is_some()
            || profile
                .strict_convex_line_clipped_side_overlap(width, depth)
                .is_some()
            || profile
                .strict_convex_arc_only_clipped_side_overlap(width, depth)
                .is_some()
            || profile
                .strict_convex_line_arc_clipped_side_overlap(width, depth)
                .is_some()
        {
            return render_line_clipped_side_overlap_split_mesh(profile, width, depth, height);
        }
        return render_corner_overlap_split_mesh(profile, width, depth, height);
    }
    if let Some(boolean) = request.boolean.as_ref()
        && let Some(profile) = boolean.profile.as_ref()
        && boolean.operation == BooleanOperation::Split
        && (profile.is_line_arc_d_profile()
            || profile.is_line_arc_capsule_profile()
            || profile.is_line_arc_rounded_rectangle_profile()
            || profile.is_strict_convex_line_arc_profile())
    {
        let (mut vertices, mut triangles) =
            render_polygon_cut_mesh(width, depth, height, profile, None)?;
        let (inner_vertices, mut inner_triangles) = render_mixed_profile_mesh(profile, height)?;
        let offset = vertices.len() as u32;
        for triangle in &mut inner_triangles {
            triangle.vertex_indices = triangle.vertex_indices.map(|index| index + offset);
        }
        vertices.extend(inner_vertices);
        triangles.extend(inner_triangles);
        return Ok((vertices, triangles));
    }
    if curved_corner_overlap_intersection
        && let Some(profile) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
    {
        return render_clipped_mixed_profile_mesh(profile, width, depth, height);
    }
    if corner_overlap_cut
        && let Some(profile) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
    {
        return request.pocket_depth_bits.map_or_else(
            || render_corner_overlap_cut_mesh(profile, width, depth, height),
            |depth_bits| {
                render_side_clipped_overlap_pocket_mesh(
                    profile,
                    width,
                    depth,
                    height,
                    f64::from_bits(depth_bits),
                )
            },
        );
    }
    if side_clipped_union
        && let Some(profile) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
    {
        return render_side_clipped_overlap_union_mesh(profile, width, depth, height);
    }
    if side_clipped_cut
        && let Some(profile) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
    {
        return request.pocket_depth_bits.map_or_else(
            || render_line_clipped_side_overlap_cut_mesh(profile, width, depth, height),
            |depth_bits| {
                render_side_clipped_overlap_pocket_mesh(
                    profile,
                    width,
                    depth,
                    height,
                    f64::from_bits(depth_bits),
                )
            },
        );
    }
    if let Some(boolean) = request.boolean.as_ref()
        && let Some(profile) = boolean.profile.as_ref()
        && boolean.operation != BooleanOperation::Split
        && !side_overlap_intersection
        && !side_overlap_cut
    {
        return if matches!(
            boolean.operation,
            BooleanOperation::Union | BooleanOperation::Intersect
        ) {
            render_mixed_profile_mesh(profile, height)
        } else {
            render_polygon_cut_mesh(
                width,
                depth,
                height,
                profile,
                request.pocket_depth_bits.map(f64::from_bits),
            )
        };
    }
    if let Some(boolean) = request.boolean.as_ref().filter(|boolean| {
        boolean.circle.is_some()
            && (boolean.operation != BooleanOperation::Split
                || boolean.circle.is_some_and(|circle| {
                    circle.side_overlap(width, depth).is_some()
                        || circle.corner_overlap(width, depth).is_some()
                        || circle.outside_side_overlap(width, depth).is_some()
                        || circle.center_on_side_overlap(width, depth).is_some()
                        || circle.center_on_corner_overlap(width, depth).is_some()
                        || circle.outside_corner_overlap(width, depth).is_some()
                }))
    }) {
        let circle = boolean.circle.expect("filtered circular boolean");
        return if boolean.operation == BooleanOperation::Split
            && (circle.corner_overlap(width, depth).is_some()
                || circle.center_on_corner_overlap(width, depth).is_some()
                || circle.outside_corner_overlap(width, depth).is_some())
        {
            render_corner_overlapping_circular_split_mesh(width, depth, height, circle)
        } else if boolean.operation == BooleanOperation::Split
            && (circle.side_overlap(width, depth).is_some()
                || circle.outside_side_overlap(width, depth).is_some()
                || circle.center_on_side_overlap(width, depth).is_some())
        {
            render_side_overlapping_circular_split_mesh(width, depth, height, circle)
        } else if boolean.operation == BooleanOperation::Union
            && (circle.corner_overlap(width, depth).is_some()
                || circle.center_on_corner_overlap(width, depth).is_some()
                || circle.outside_corner_overlap(width, depth).is_some())
        {
            render_corner_overlapping_circular_union_mesh(width, depth, height, circle)
        } else if boolean.operation == BooleanOperation::Union
            && (circle.side_overlap(width, depth).is_some()
                || circle.outside_side_overlap(width, depth).is_some()
                || circle.center_on_side_overlap(width, depth).is_some())
        {
            render_side_overlapping_circular_union_mesh(width, depth, height, circle)
        } else if boolean.operation == BooleanOperation::Intersect
            && (circle.corner_overlap(width, depth).is_some()
                || circle.center_on_corner_overlap(width, depth).is_some()
                || circle.outside_corner_overlap(width, depth).is_some())
        {
            render_corner_overlapping_circular_intersection_mesh(width, depth, height, circle)
        } else if boolean.operation == BooleanOperation::Intersect
            && (circle.side_overlap(width, depth).is_some()
                || circle.outside_side_overlap(width, depth).is_some()
                || circle.center_on_side_overlap(width, depth).is_some())
        {
            render_side_overlapping_circular_intersection_mesh(width, depth, height, circle)
        } else if matches!(
            boolean.operation,
            BooleanOperation::Union | BooleanOperation::Intersect
        ) {
            render_circle_mesh(circle, height)
        } else if let Some(pocket_depth_bits) = request.pocket_depth_bits
            && (circle.side_overlap(width, depth).is_some()
                || circle.corner_overlap(width, depth).is_some()
                || circle.outside_side_overlap(width, depth).is_some()
                || circle.center_on_side_overlap(width, depth).is_some()
                || circle.center_on_corner_overlap(width, depth).is_some()
                || circle.outside_corner_overlap(width, depth).is_some())
        {
            render_overlapping_circular_pocket_mesh(
                width,
                depth,
                height,
                f64::from_bits(pocket_depth_bits),
                circle,
            )
        } else {
            render_circular_cut_mesh(width, depth, height, circle)
        };
    }
    if [width, depth, height]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0)
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let [min, max] = request.expected_bounds_mm();
    let outer = [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [max[0], max[1], max[2]],
        [min[0], max[1], max[2]],
    ];
    let Some(cut) = request
        .boolean
        .as_ref()
        .filter(|boolean| boolean.operation == BooleanOperation::Cut && !side_overlap_cut)
    else {
        let vertices = outer
            .map(|position_mm| ExactVertex { position_mm })
            .to_vec();
        let (bottom_role, top_role, side_roles) = if request.shell.is_some() {
            (
                ExactFaceRole::BoxShellOuterBottom,
                ExactFaceRole::BoxShellRim,
                [Some(ExactFaceRole::BoxShellOuterEast), None, None, None],
            )
        } else if side_overlap_intersection {
            (
                ExactFaceRole::Bottom,
                ExactFaceRole::Top,
                [Some(ExactFaceRole::LinearSide), None, None, None],
            )
        } else if side_overlap_cut {
            let side_roles = if max[0] < width {
                [
                    Some(ExactFaceRole::CutLinear),
                    Some(ExactFaceRole::West),
                    None,
                    None,
                ]
            } else if min[0] > 0.0 {
                [
                    Some(ExactFaceRole::East),
                    Some(ExactFaceRole::CutLinear),
                    None,
                    None,
                ]
            } else if max[1] < depth {
                [
                    Some(ExactFaceRole::East),
                    None,
                    Some(ExactFaceRole::CutLinear),
                    None,
                ]
            } else {
                [
                    Some(ExactFaceRole::East),
                    None,
                    None,
                    Some(ExactFaceRole::CutLinear),
                ]
            };
            (ExactFaceRole::Bottom, ExactFaceRole::Top, side_roles)
        } else {
            (
                ExactFaceRole::Bottom,
                ExactFaceRole::Top,
                [Some(ExactFaceRole::East), None, None, None],
            )
        };
        let triangles = [
            ([0, 2, 1], Some(bottom_role)),
            ([0, 3, 2], Some(bottom_role)),
            ([4, 5, 6], Some(top_role)),
            ([4, 6, 7], Some(top_role)),
            ([1, 2, 6], side_roles[0]),
            ([1, 6, 5], side_roles[0]),
            ([0, 4, 7], side_roles[1]),
            ([0, 7, 3], side_roles[1]),
            ([3, 7, 6], side_roles[2]),
            ([3, 6, 2], side_roles[2]),
            ([0, 1, 5], side_roles[3]),
            ([0, 5, 4], side_roles[3]),
        ]
        .map(|(vertex_indices, face_role)| ExactTriangle {
            vertex_indices,
            face_role,
        })
        .to_vec();
        return Ok((vertices, triangles));
    };
    let x0 = f64::from_bits(cut.min_x_bits);
    let y0 = f64::from_bits(cut.min_y_bits);
    let x1 = x0 + f64::from_bits(cut.width_bits);
    let y1 = y0 + f64::from_bits(cut.depth_bits);
    if [x0, y0, x1, y1].into_iter().any(|value| !value.is_finite())
        || x0 <= 0.0
        || y0 <= 0.0
        || x1 >= width
        || y1 >= depth
        || x0 >= x1
        || y0 >= y1
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let pocket_floor_z = request
        .pocket_depth_bits
        .map_or(0.0, |bits| height - f64::from_bits(bits));
    if !pocket_floor_z.is_finite() || pocket_floor_z < 0.0 || pocket_floor_z >= height {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut positions = outer.to_vec();
    positions.extend([
        [x0, y0, pocket_floor_z],
        [x1, y0, pocket_floor_z],
        [x1, y1, pocket_floor_z],
        [x0, y1, pocket_floor_z],
        [x0, y0, height],
        [x1, y0, height],
        [x1, y1, height],
        [x0, y1, height],
    ]);
    let vertices = positions
        .into_iter()
        .map(|position_mm| ExactVertex { position_mm })
        .collect();
    let mut triangles = Vec::with_capacity(32);
    let mut quad = |indices: [u32; 4], role| {
        triangles.push(ExactTriangle {
            vertex_indices: [indices[0], indices[1], indices[2]],
            face_role: role,
        });
        triangles.push(ExactTriangle {
            vertex_indices: [indices[0], indices[2], indices[3]],
            face_role: role,
        });
    };
    if request.pocket_depth_bits.is_some() {
        quad([0, 3, 2, 1], Some(ExactFaceRole::Bottom));
        for indices in [
            [4, 5, 13, 12],
            [5, 6, 14, 13],
            [6, 7, 15, 14],
            [7, 4, 12, 15],
        ] {
            quad(indices, Some(ExactFaceRole::Top));
        }
        quad([0, 1, 5, 4], None);
        quad([1, 2, 6, 5], Some(ExactFaceRole::East));
        quad([2, 3, 7, 6], None);
        quad([3, 0, 4, 7], None);
        quad([8, 9, 10, 11], Some(ExactFaceRole::PocketFloor));
        quad([11, 15, 12, 8], Some(ExactFaceRole::PocketWest));
        quad([9, 13, 14, 10], Some(ExactFaceRole::PocketEast));
        quad([8, 12, 13, 9], Some(ExactFaceRole::PocketSouth));
        quad([10, 14, 15, 11], Some(ExactFaceRole::PocketNorth));
        return Ok((vertices, triangles));
    }
    for indices in [[0, 8, 9, 1], [1, 9, 10, 2], [2, 10, 11, 3], [3, 11, 8, 0]] {
        quad(indices, Some(ExactFaceRole::Bottom));
    }
    for indices in [
        [4, 5, 13, 12],
        [5, 6, 14, 13],
        [6, 7, 15, 14],
        [7, 4, 12, 15],
    ] {
        quad(indices, Some(ExactFaceRole::Top));
    }
    quad([0, 1, 5, 4], None);
    quad([1, 2, 6, 5], Some(ExactFaceRole::East));
    quad([2, 3, 7, 6], None);
    quad([3, 0, 4, 7], None);
    quad([11, 15, 12, 8], Some(ExactFaceRole::CutWest));
    quad([9, 13, 14, 10], Some(ExactFaceRole::CutEast));
    quad([8, 12, 13, 9], Some(ExactFaceRole::CutSouth));
    quad([10, 14, 15, 11], Some(ExactFaceRole::CutNorth));
    Ok((vertices, triangles))
}

fn render_polygon_cut_mesh(
    width: f64,
    depth: f64,
    height: f64,
    profile: &ExactMixedProfile,
    pocket_depth_mm: Option<f64>,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let is_arc_cut_profile = profile.is_line_arc_d_profile()
        || profile.is_line_arc_capsule_profile()
        || profile.is_line_arc_rounded_rectangle_profile()
        || profile.is_strict_convex_line_arc_profile();
    if [width, depth, height]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0)
        || (!profile.has_only_line_segments() && !is_arc_cut_profile)
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let pocket_floor_z = match pocket_depth_mm {
        Some(pocket_depth)
            if pocket_depth.is_finite() && pocket_depth > 0.0 && pocket_depth < height =>
        {
            height - pocket_depth
        }
        Some(_) => return Err(ExactProductError::InvalidWorkerEvidence),
        None => 0.0,
    };
    let mut hole = Vec::new();
    let mut linear_edges = Vec::new();
    for (segment_index, segment) in profile.segments.iter().enumerate() {
        let (start, end, center, sweep, steps, is_linear) = match segment {
            ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } => (
                start_bits.map(f64::from_bits),
                end_bits.map(f64::from_bits),
                [0.0; 2],
                0.0,
                1,
                true,
            ),
            ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } if is_arc_cut_profile => {
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)
                    .ok_or(ExactProductError::InvalidWorkerEvidence)?;
                let steps = (sweep.abs() / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
                (start, end, center, sweep, steps, false)
            }
            ExactProfileSegment::CircularArc { .. } => {
                return Err(ExactProductError::InvalidWorkerEvidence);
            }
        };
        let closure_tolerance = if sweep == 0.0 {
            1.0e-9
        } else {
            1.0e-9 * (start[0] - center[0]).hypot(start[1] - center[1]).max(1.0)
        };
        if start
            .into_iter()
            .chain(end)
            .chain(center)
            .any(|value| !value.is_finite())
            || (!hole.is_empty() && hole.last() != Some(&start))
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        if hole.is_empty() {
            hole.push(start);
        }
        for step in 1..=steps {
            let point = if is_linear || step == steps {
                end
            } else {
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let angle = start_angle + sweep * step as f64 / steps as f64;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            };
            if point[0] <= 0.0 || point[1] <= 0.0 || point[0] >= width || point[1] >= depth {
                return Err(ExactProductError::InvalidWorkerEvidence);
            }
            linear_edges.push(is_linear);
            let closes = segment_index + 1 == profile.segments.len()
                && step == steps
                && point
                    .into_iter()
                    .zip(hole[0])
                    .all(|(actual, expected)| (actual - expected).abs() <= closure_tolerance);
            if !closes {
                hole.push(point);
            }
        }
    }
    if hole.len() < 3
        || linear_edges.len() != hole.len()
        || polygon_signed_area(&hole).abs() <= 1.0e-12
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    if polygon_signed_area(&hole) > 0.0 {
        hole.reverse();
        linear_edges.reverse();
        linear_edges.rotate_left(1);
    }
    let bridge_index = hole
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            left[0]
                .total_cmp(&right[0])
                .then_with(|| left[1].total_cmp(&right[1]))
        })
        .map(|(index, _)| index)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    hole.rotate_left(bridge_index);
    linear_edges.rotate_left(bridge_index);
    let bridge = hole[0];
    let epsilon = width.min(depth) * 1.0e-10;
    let mut cap_boundary = vec![
        [width, bridge[1] + epsilon],
        [width, depth],
        [0.0, depth],
        [0.0, 0.0],
        [width, 0.0],
        [width, bridge[1] - epsilon],
        [bridge[0], bridge[1] - epsilon],
    ];
    cap_boundary.extend(hole.iter().copied().skip(1));
    cap_boundary.push([bridge[0], bridge[1] + epsilon]);
    let cap_triangles = triangulate_polygon(&cap_boundary)?;
    let cap_orientation = polygon_signed_area(&cap_boundary);
    let mut vertices = Vec::new();
    for point in &cap_boundary {
        vertices.push(ExactVertex {
            position_mm: [point[0], point[1], 0.0],
        });
        vertices.push(ExactVertex {
            position_mm: [point[0], point[1], height],
        });
    }
    let mut triangles = Vec::new();
    for [a, b, c] in cap_triangles {
        let top = if cap_orientation > 0.0 {
            [a, b, c]
        } else {
            [a, c, b]
        };
        triangles.push(ExactTriangle {
            vertex_indices: [top[0] * 2 + 1, top[1] * 2 + 1, top[2] * 2 + 1],
            face_role: Some(ExactFaceRole::Top),
        });
        if pocket_depth_mm.is_none() {
            triangles.push(ExactTriangle {
                vertex_indices: [top[0] * 2, top[2] * 2, top[1] * 2],
                face_role: Some(ExactFaceRole::Bottom),
            });
        }
    }
    if pocket_depth_mm.is_some() {
        for [a, b, c] in [[3, 0, 4], [3, 1, 0], [3, 2, 1]] {
            triangles.push(ExactTriangle {
                vertex_indices: [a * 2, b * 2, c * 2],
                face_role: Some(ExactFaceRole::Bottom),
            });
        }
    }
    for index in 0..5_u32 {
        let next = (index + 1) % 5;
        let bottom = index * 2;
        let top = bottom + 1;
        let next_bottom = next * 2;
        let next_top = next_bottom + 1;
        let role = (index == 0).then_some(ExactFaceRole::East);
        triangles.extend([
            ExactTriangle {
                vertex_indices: [bottom, next_bottom, next_top],
                face_role: role,
            },
            ExactTriangle {
                vertex_indices: [bottom, next_top, top],
                face_role: role,
            },
        ]);
    }
    let wall_start = vertices.len() as u32;
    for point in &hole {
        vertices.push(ExactVertex {
            position_mm: [point[0], point[1], pocket_floor_z],
        });
        vertices.push(ExactVertex {
            position_mm: [point[0], point[1], height],
        });
    }
    if pocket_depth_mm.is_some() {
        let floor_orientation = polygon_signed_area(&hole);
        for [a, b, c] in triangulate_polygon(&hole)? {
            let floor = if floor_orientation > 0.0 {
                [a, b, c]
            } else {
                [a, c, b]
            };
            triangles.push(ExactTriangle {
                vertex_indices: [
                    wall_start + floor[0] * 2,
                    wall_start + floor[1] * 2,
                    wall_start + floor[2] * 2,
                ],
                face_role: Some(ExactFaceRole::PocketFloor),
            });
        }
    }
    for (index, linear_edge) in linear_edges.iter().copied().enumerate() {
        let next = (index + 1) % hole.len();
        let bottom = wall_start + (index * 2) as u32;
        let top = bottom + 1;
        let next_bottom = wall_start + (next * 2) as u32;
        let next_top = next_bottom + 1;
        let role = if is_arc_cut_profile {
            linear_edge.then_some(ExactFaceRole::CutLinear)
        } else {
            (index == 0).then_some(ExactFaceRole::CutLinear)
        };
        triangles.extend([
            ExactTriangle {
                vertex_indices: [bottom, next_top, next_bottom],
                face_role: role,
            },
            ExactTriangle {
                vertex_indices: [bottom, top, next_top],
                face_role: role,
            },
        ]);
    }
    let weld_tolerance = epsilon * 3.0;
    let mut welded_vertices: Vec<ExactVertex> = Vec::new();
    let remap = vertices
        .into_iter()
        .map(|vertex| {
            welded_vertices
                .iter()
                .position(|candidate| {
                    candidate.position_mm[2].to_bits() == vertex.position_mm[2].to_bits()
                        && (candidate.position_mm[0] - vertex.position_mm[0]).abs()
                            <= weld_tolerance
                        && (candidate.position_mm[1] - vertex.position_mm[1]).abs()
                            <= weld_tolerance
                })
                .map_or_else(
                    || {
                        welded_vertices.push(vertex);
                        (welded_vertices.len() - 1) as u32
                    },
                    |index| index as u32,
                )
        })
        .collect::<Vec<_>>();
    let welded_triangles = triangles
        .into_iter()
        .filter_map(|mut triangle| {
            triangle.vertex_indices = triangle.vertex_indices.map(|index| remap[index as usize]);
            let [a, b, c] = triangle.vertex_indices;
            if a == b || b == c || c == a {
                return None;
            }
            let [a, b, c] = [a, b, c].map(|index| welded_vertices[index as usize].position_mm);
            let first = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let second = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                first[1] * second[2] - first[2] * second[1],
                first[2] * second[0] - first[0] * second[2],
                first[0] * second[1] - first[1] * second[0],
            ];
            cross
                .into_iter()
                .map(|value| value * value)
                .sum::<f64>()
                .gt(&1.0e-12)
                .then_some(triangle)
        })
        .collect();
    Ok((welded_vertices, welded_triangles))
}

fn render_mixed_profile_mesh(
    profile: &ExactMixedProfile,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if !height.is_finite() || height <= 0.0 {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut boundary = Vec::<[f64; 2]>::new();
    let mut reference_side_edges = Vec::<bool>::new();
    let first_arc = profile
        .segments
        .iter()
        .position(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }));
    let reference_segment = first_arc.unwrap_or(0);
    let reference_role = first_arc.map_or(ExactFaceRole::LinearSide, |_| ExactFaceRole::ArcSide);
    for (segment_index, segment) in profile.segments.iter().enumerate() {
        let (start, end, center, sweep, steps) = match segment {
            ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } => (
                start_bits.map(f64::from_bits),
                end_bits.map(f64::from_bits),
                [0.0; 2],
                0.0,
                1,
            ),
            ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => {
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)
                    .ok_or(ExactProductError::InvalidWorkerEvidence)?;
                let steps = (sweep.abs() / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
                (start, end, center, sweep, steps)
            }
        };
        let closure_tolerance = if sweep == 0.0 {
            1.0e-9
        } else {
            1.0e-9 * (start[0] - center[0]).hypot(start[1] - center[1]).max(1.0)
        };
        if boundary.is_empty() {
            boundary.push(start);
        } else if boundary.last() != Some(&start) {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        for step in 1..=steps {
            let point = if sweep == 0.0 || step == steps {
                end
            } else {
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let angle = start_angle + sweep * step as f64 / steps as f64;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            };
            reference_side_edges.push(segment_index == reference_segment);
            let closes_boundary = segment_index + 1 == profile.segments.len()
                && step == steps
                && point
                    .into_iter()
                    .zip(boundary[0])
                    .all(|(actual, expected)| (actual - expected).abs() <= closure_tolerance);
            if !closes_boundary {
                boundary.push(point);
            }
        }
    }
    if boundary.len() < 3 || reference_side_edges.len() != boundary.len() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let cap_triangles = triangulate_polygon(&boundary)?;
    let mut vertices = Vec::with_capacity(boundary.len() * 2);
    for point in &boundary {
        vertices.push(ExactVertex {
            position_mm: [point[0], point[1], 0.0],
        });
        vertices.push(ExactVertex {
            position_mm: [point[0], point[1], height],
        });
    }
    let signed_area = polygon_signed_area(&boundary);
    let mut triangles = Vec::with_capacity(cap_triangles.len() * 2 + boundary.len() * 2);
    for [a, b, c] in cap_triangles {
        let top = if signed_area > 0.0 {
            [a, b, c]
        } else {
            [a, c, b]
        };
        triangles.push(ExactTriangle {
            vertex_indices: [top[0] * 2 + 1, top[1] * 2 + 1, top[2] * 2 + 1],
            face_role: Some(ExactFaceRole::Top),
        });
        triangles.push(ExactTriangle {
            vertex_indices: [top[0] * 2, top[2] * 2, top[1] * 2],
            face_role: Some(ExactFaceRole::Bottom),
        });
    }
    for (index, reference_side) in reference_side_edges.iter().copied().enumerate() {
        let next = (index + 1) % boundary.len();
        let bottom = (index * 2) as u32;
        let top = bottom + 1;
        let next_bottom = (next * 2) as u32;
        let next_top = next_bottom + 1;
        let role = reference_side.then_some(reference_role);
        triangles.extend([
            ExactTriangle {
                vertex_indices: [bottom, next_bottom, next_top],
                face_role: role,
            },
            ExactTriangle {
                vertex_indices: [bottom, next_top, top],
                face_role: role,
            },
        ]);
    }
    Ok((vertices, triangles))
}

fn clip_polygon_to_half_plane(
    boundary: Vec<[f64; 2]>,
    axis: usize,
    limit: f64,
    keep_greater: bool,
) -> Vec<[f64; 2]> {
    let Some(mut previous) = boundary.last().copied() else {
        return boundary;
    };
    let mut previous_inside = if keep_greater {
        previous[axis] >= limit - 1.0e-9
    } else {
        previous[axis] <= limit + 1.0e-9
    };
    let mut clipped = Vec::new();
    for current in boundary {
        let current_inside = if keep_greater {
            current[axis] >= limit - 1.0e-9
        } else {
            current[axis] <= limit + 1.0e-9
        };
        if current_inside != previous_inside {
            let denominator = current[axis] - previous[axis];
            if denominator.abs() > 1.0e-12 {
                let t = (limit - previous[axis]) / denominator;
                let mut intersection = [
                    previous[0] + t * (current[0] - previous[0]),
                    previous[1] + t * (current[1] - previous[1]),
                ];
                intersection[axis] = limit;
                clipped.push(intersection);
            }
        }
        if current_inside {
            clipped.push(current);
        }
        previous = current;
        previous_inside = current_inside;
    }
    clipped
}

fn render_clipped_mixed_profile_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if !width.is_finite()
        || !depth.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || depth <= 0.0
        || height <= 0.0
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut boundary = Vec::<[f64; 2]>::new();
    for (segment_index, segment) in profile.segments.iter().enumerate() {
        let (start, end, center, sweep, steps) = match segment {
            ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } => (
                start_bits.map(f64::from_bits),
                end_bits.map(f64::from_bits),
                [0.0; 2],
                0.0,
                1,
            ),
            ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => {
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)
                    .ok_or(ExactProductError::InvalidWorkerEvidence)?;
                let steps = (sweep.abs() / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
                (start, end, center, sweep, steps)
            }
        };
        if boundary.is_empty() {
            boundary.push(start);
        } else if boundary.last().is_none_or(|previous| {
            previous
                .iter()
                .zip(start)
                .any(|(actual, expected)| (actual - expected).abs() > 1.0e-9)
        }) {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        for step in 1..=steps {
            let point = if sweep == 0.0 || step == steps {
                end
            } else {
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let angle = start_angle + sweep * step as f64 / steps as f64;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            };
            let closes_boundary = segment_index + 1 == profile.segments.len()
                && step == steps
                && point
                    .into_iter()
                    .zip(boundary[0])
                    .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-9);
            if !closes_boundary {
                boundary.push(point);
            }
        }
    }
    for (axis, limit, keep_greater) in [
        (0, 0.0, true),
        (0, width, false),
        (1, 0.0, true),
        (1, depth, false),
    ] {
        boundary = clip_polygon_to_half_plane(boundary, axis, limit, keep_greater);
    }
    boundary.dedup_by(|left, right| {
        left.iter()
            .zip(right.iter())
            .all(|(left, right)| (*left - *right).abs() <= 1.0e-9)
    });
    if boundary.len() >= 2
        && boundary[0]
            .into_iter()
            .zip(*boundary.last().expect("non-empty clipped boundary"))
            .all(|(left, right)| (left - right).abs() <= 1.0e-9)
    {
        boundary.pop();
    }
    if boundary.len() < 3 {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let (arc_center, arc_radius) = profile
        .segments
        .iter()
        .filter_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((center, (start[0] - center[0]).hypot(start[1] - center[1])))
            }
            ExactProfileSegment::Line { .. } => None,
        })
        .find(|(center, radius)| {
            let tolerance = radius.max(1.0) * 1.0e-6;
            boundary.iter().enumerate().any(|(index, start)| {
                let end = boundary[(index + 1) % boundary.len()];
                [*start, end].into_iter().all(|point| {
                    ((point[0] - center[0]).hypot(point[1] - center[1]) - radius).abs() <= tolerance
                })
            })
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let arc_tolerance = arc_radius.max(1.0) * 1.0e-6;
    let reference_side_edges = boundary
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = boundary[(index + 1) % boundary.len()];
            let on_arc = |point: [f64; 2]| {
                ((point[0] - arc_center[0]).hypot(point[1] - arc_center[1]) - arc_radius).abs()
                    <= arc_tolerance
            };
            on_arc(*start) && on_arc(end)
        })
        .collect::<Vec<_>>();
    if !reference_side_edges.iter().any(|reference| *reference) {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let cap_triangles = triangulate_polygon(&boundary)?;
    let mut vertices = Vec::with_capacity(boundary.len() * 2);
    for point in &boundary {
        vertices.extend([
            ExactVertex {
                position_mm: [point[0], point[1], 0.0],
            },
            ExactVertex {
                position_mm: [point[0], point[1], height],
            },
        ]);
    }
    let signed_area = polygon_signed_area(&boundary);
    let mut triangles = Vec::with_capacity(cap_triangles.len() * 2 + boundary.len() * 2);
    for [a, b, c] in cap_triangles {
        let top = if signed_area > 0.0 {
            [a, b, c]
        } else {
            [a, c, b]
        };
        triangles.extend([
            ExactTriangle {
                vertex_indices: [top[0] * 2 + 1, top[1] * 2 + 1, top[2] * 2 + 1],
                face_role: Some(ExactFaceRole::Top),
            },
            ExactTriangle {
                vertex_indices: [top[0] * 2, top[2] * 2, top[1] * 2],
                face_role: Some(ExactFaceRole::Bottom),
            },
        ]);
    }
    for (index, reference_side) in reference_side_edges.into_iter().enumerate() {
        let next = (index + 1) % boundary.len();
        let bottom = (index * 2) as u32;
        let top = bottom + 1;
        let next_bottom = (next * 2) as u32;
        let next_top = next_bottom + 1;
        let role = reference_side.then_some(ExactFaceRole::ArcSide);
        triangles.extend([
            ExactTriangle {
                vertex_indices: [bottom, next_bottom, next_top],
                face_role: role,
            },
            ExactTriangle {
                vertex_indices: [bottom, next_top, top],
                face_role: role,
            },
        ]);
    }
    Ok((vertices, triangles))
}

type ClippedProfileBoundary = (Vec<[f64; 2]>, [f64; 2], f64);

fn render_side_clipped_overlap_union_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile.capsule_corner_overlap(width, depth).is_some()
        || profile
            .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
            .is_some()
        || profile
            .strict_convex_line_arc_clipped_south_west_corner_overlap(width, depth)
            .is_some()
        || profile
            .strict_convex_line_arc_clipped_north_east_corner_overlap(width, depth)
            .is_some()
        || profile
            .strict_convex_line_arc_clipped_north_west_corner_overlap(width, depth)
            .is_some()
    {
        return render_corner_overlap_union_mesh(profile, width, depth, height);
    }
    if profile.capsule_side_overlap(width, depth).is_some() {
        return render_capsule_side_overlap_union_mesh(profile, width, depth, height);
    }
    if profile
        .rounded_rectangle_chord_side_overlap(width, depth)
        .is_some()
    {
        return render_chord_side_overlap_union_mesh(profile, width, depth, height);
    }
    if profile
        .strict_convex_arc_only_clipped_side_overlap(width, depth)
        .is_some()
        || profile
            .d_profile_arc_only_clipped_side_overlap(width, depth)
            .is_some()
    {
        return render_arc_only_clipped_side_overlap_union_mesh(profile, width, depth, height);
    }
    if profile
        .strict_convex_line_arc_clipped_side_overlap(width, depth)
        .is_some()
    {
        return render_line_arc_clipped_side_overlap_union_mesh(profile, width, depth, height);
    }
    if profile
        .strict_convex_line_clipped_side_overlap(width, depth)
        .is_none()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
    let tolerance = 1.0e-6;
    let mut boundary = if min_x < -tolerance
        && max_x < width - tolerance
        && min_y > tolerance
        && max_y < depth - tolerance
    {
        vec![
            [0.0, 0.0],
            [width, 0.0],
            [width, depth],
            [0.0, depth],
            [0.0, max_y],
            [min_x, max_y],
            [min_x, min_y],
            [0.0, min_y],
        ]
    } else if max_x > width + tolerance
        && min_x > tolerance
        && min_y > tolerance
        && max_y < depth - tolerance
    {
        vec![
            [0.0, 0.0],
            [width, 0.0],
            [width, min_y],
            [max_x, min_y],
            [max_x, max_y],
            [width, max_y],
            [width, depth],
            [0.0, depth],
        ]
    } else if min_y < -tolerance
        && max_y < depth - tolerance
        && min_x > tolerance
        && max_x < width - tolerance
    {
        vec![
            [0.0, 0.0],
            [min_x, 0.0],
            [min_x, min_y],
            [max_x, min_y],
            [max_x, 0.0],
            [width, 0.0],
            [width, depth],
            [0.0, depth],
        ]
    } else if max_y > depth + tolerance
        && min_y > tolerance
        && min_x > tolerance
        && max_x < width - tolerance
    {
        vec![
            [0.0, 0.0],
            [width, 0.0],
            [width, depth],
            [max_x, depth],
            [max_x, max_y],
            [min_x, max_y],
            [min_x, depth],
            [0.0, depth],
        ]
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    render_profile_boundary_prism(&boundary, height, None)
}

fn render_chord_side_overlap_union_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile
        .rounded_rectangle_chord_side_overlap(width, depth)
        .is_none()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let tolerance = 1.0e-6;
    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
    let (axis, limit, ascending) = if min_x < -tolerance {
        (0_usize, 0.0, false)
    } else if max_x > width + tolerance {
        (0, width, true)
    } else if min_y < -tolerance {
        (1, 0.0, true)
    } else if max_y > depth + tolerance {
        (1, depth, false)
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let outside = |point: [f64; 2]| {
        if limit == 0.0 {
            point[axis] < limit - tolerance
        } else {
            point[axis] > limit + tolerance
        }
    };
    let same_point = |left: [f64; 2], right: [f64; 2]| {
        left.into_iter()
            .zip(right)
            .all(|(left, right)| (left - right).abs() <= tolerance)
    };

    let (profile_vertices, _) = render_mixed_profile_mesh(profile, height)?;
    let mut profile_boundary = profile_vertices
        .chunks_exact(2)
        .map(|pair| {
            let [x, y, _] = pair[0].position_mm;
            [x, y]
        })
        .collect::<Vec<_>>();
    let mut contacts = Vec::with_capacity(2);
    for index in 0..profile_boundary.len() {
        let start = profile_boundary[index];
        let end = profile_boundary[(index + 1) % profile_boundary.len()];
        if outside(start) == outside(end) {
            continue;
        }
        let denominator = end[axis] - start[axis];
        if denominator.abs() <= tolerance {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let parameter = (limit - start[axis]) / denominator;
        if parameter <= tolerance || parameter >= 1.0 - tolerance {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let mut contact = [
            start[0] + parameter * (end[0] - start[0]),
            start[1] + parameter * (end[1] - start[1]),
        ];
        contact[axis] = limit;
        contacts.push(contact);
    }
    let [first_contact, second_contact] = contacts
        .as_slice()
        .try_into()
        .map_err(|_| ExactProductError::InvalidWorkerEvidence)?;
    let insert_contact = |boundary: &mut Vec<[f64; 2]>, contact: [f64; 2]| {
        if boundary.iter().any(|point| same_point(*point, contact)) {
            return Ok(());
        }
        let edge = (0..boundary.len())
            .find(|index| {
                let start = boundary[*index];
                let end = boundary[(*index + 1) % boundary.len()];
                outside(start) != outside(end) && (start[axis] - limit) * (end[axis] - limit) < 0.0
            })
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        boundary.insert(edge + 1, contact);
        Ok(())
    };
    insert_contact(&mut profile_boundary, first_contact)?;
    insert_contact(&mut profile_boundary, second_contact)?;
    let first_index = profile_boundary
        .iter()
        .position(|point| same_point(*point, first_contact))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let second_index = profile_boundary
        .iter()
        .position(|point| same_point(*point, second_contact))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let cyclic_path = |start: usize, end: usize| {
        let mut path = Vec::new();
        let mut index = start;
        loop {
            path.push(profile_boundary[index]);
            if index == end {
                break;
            }
            index = (index + 1) % profile_boundary.len();
        }
        path
    };
    let first_path = cyclic_path(first_index, second_index);
    let second_path = cyclic_path(second_index, first_index);
    let outside_count = |path: &[[f64; 2]]| path.iter().filter(|point| outside(**point)).count();
    let mut outside_path = match outside_count(&first_path).cmp(&outside_count(&second_path)) {
        std::cmp::Ordering::Greater => first_path,
        std::cmp::Ordering::Less => second_path,
        std::cmp::Ordering::Equal => return Err(ExactProductError::InvalidWorkerEvidence),
    };
    let orthogonal_axis = 1 - axis;
    if (outside_path[0][orthogonal_axis] < outside_path[outside_path.len() - 1][orthogonal_axis])
        != ascending
    {
        outside_path.reverse();
    }
    let mut boundary = outside_path;
    match (axis, limit == 0.0) {
        (0, false) => boundary.extend([[width, depth], [0.0, depth], [0.0, 0.0], [width, 0.0]]),
        (0, true) => boundary.extend([[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]]),
        (1, false) => boundary.extend([[0.0, depth], [0.0, 0.0], [width, 0.0], [width, depth]]),
        (1, true) => boundary.extend([[width, 0.0], [width, depth], [0.0, depth], [0.0, 0.0]]),
        _ => unreachable!("validated clipping axis"),
    }
    boundary.dedup_by(|left, right| same_point(*left, *right));
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let arc = profile
        .segments
        .iter()
        .find_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                (outside(start) && outside(end))
                    .then_some((center, (start[0] - center[0]).hypot(start[1] - center[1])))
            }
            ExactProfileSegment::Line { .. } => None,
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    render_profile_boundary_prism(&boundary, height, Some(arc))
}

fn render_capsule_side_overlap_union_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile.capsule_side_overlap(width, depth).is_none() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let tolerance = 1.0e-6;
    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
    let (axis, limit, ascending) = if min_x < -tolerance {
        (0_usize, 0.0, false)
    } else if max_x > width + tolerance {
        (0, width, true)
    } else if min_y < -tolerance {
        (1, 0.0, true)
    } else if max_y > depth + tolerance {
        (1, depth, false)
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let outside = |point: [f64; 2]| {
        if limit == 0.0 {
            point[axis] < limit - tolerance
        } else {
            point[axis] > limit + tolerance
        }
    };
    let same_point = |left: [f64; 2], right: [f64; 2]| {
        left.into_iter()
            .zip(right)
            .all(|(left, right)| (left - right).abs() <= tolerance)
    };
    let mut outside_arcs = profile.segments.iter().filter_map(|segment| {
        let ExactProfileSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } = segment
        else {
            return None;
        };
        let start = start_bits.map(f64::from_bits);
        let end = end_bits.map(f64::from_bits);
        (outside(start) && outside(end)).then_some((
            start,
            end,
            center_bits.map(f64::from_bits),
            *clockwise,
        ))
    });
    let (arc_start, arc_end, center, clockwise) = outside_arcs
        .next()
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    if outside_arcs.next().is_some() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut crossing_lines = profile.segments.iter().filter_map(|segment| {
        let ExactProfileSegment::Line {
            start_bits,
            end_bits,
        } = segment
        else {
            return None;
        };
        let start = start_bits.map(f64::from_bits);
        let end = end_bits.map(f64::from_bits);
        if outside(start) == outside(end) {
            return None;
        }
        let denominator = end[axis] - start[axis];
        if denominator.abs() <= tolerance {
            return None;
        }
        let t = (limit - start[axis]) / denominator;
        if t <= tolerance || t >= 1.0 - tolerance {
            return None;
        }
        let mut contact = [
            start[0] + t * (end[0] - start[0]),
            start[1] + t * (end[1] - start[1]),
        ];
        contact[axis] = limit;
        Some((if outside(start) { start } else { end }, contact))
    });
    let first_line = crossing_lines
        .next()
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let second_line = crossing_lines
        .next()
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    if crossing_lines.next().is_some() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let lines = [first_line, second_line];
    let contact_start = lines
        .iter()
        .find_map(|(endpoint, contact)| same_point(*endpoint, arc_start).then_some(*contact))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let contact_end = lines
        .iter()
        .find_map(|(endpoint, contact)| same_point(*endpoint, arc_end).then_some(*contact))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let radius = (arc_start[0] - center[0]).hypot(arc_start[1] - center[1]);
    let start_angle = (arc_start[1] - center[1]).atan2(arc_start[0] - center[0]);
    let end_angle = (arc_end[1] - center[1]).atan2(arc_end[0] - center[0]);
    let sweep = directed_arc_sweep(start_angle, end_angle, clockwise)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let steps = (sweep.abs() / std::f64::consts::TAU * 64.0).ceil().max(2.0) as usize;
    let mut outside_path = Vec::with_capacity(steps + 3);
    outside_path.push(contact_start);
    outside_path.extend((0..=steps).map(|step| {
        let angle = start_angle + sweep * step as f64 / steps as f64;
        [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]
    }));
    outside_path.push(contact_end);
    outside_path.first_mut().expect("non-empty outside path")[axis] = limit;
    outside_path.last_mut().expect("non-empty outside path")[axis] = limit;
    let orthogonal_axis = 1 - axis;
    if (outside_path[0][orthogonal_axis] < outside_path[outside_path.len() - 1][orthogonal_axis])
        != ascending
    {
        outside_path.reverse();
    }
    let mut boundary = match (axis, limit == 0.0) {
        (0, false) => vec![[0.0, 0.0], [width, 0.0]],
        (1, false) => vec![[0.0, 0.0], [width, 0.0], [width, depth]],
        (0, true) => vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]],
        (1, true) => vec![[0.0, 0.0]],
        _ => unreachable!("validated clipping axis"),
    };
    boundary.extend(outside_path);
    match (axis, limit == 0.0) {
        (0, false) => boundary.extend([[width, depth], [0.0, depth]]),
        (1, false) => boundary.push([0.0, depth]),
        (0, true) => {}
        (1, true) => boundary.extend([[width, 0.0], [width, depth], [0.0, depth]]),
        _ => unreachable!("validated clipping axis"),
    }
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    render_profile_boundary_prism(&boundary, height, Some((center, radius)))
}

fn render_line_arc_clipped_side_overlap_union_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile
        .strict_convex_line_arc_clipped_side_overlap(width, depth)
        .is_none()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let tolerance = 1.0e-6;
    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
    let (axis, limit, ascending) = if min_x < -tolerance {
        (0_usize, 0.0, false)
    } else if max_x > width + tolerance {
        (0, width, true)
    } else if min_y < -tolerance {
        (1, 0.0, true)
    } else if max_y > depth + tolerance {
        (1, depth, false)
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let outside = |point: [f64; 2]| {
        if limit == 0.0 {
            point[axis] < limit - tolerance
        } else {
            point[axis] > limit + tolerance
        }
    };
    let same_point = |left: [f64; 2], right: [f64; 2]| {
        left.into_iter()
            .zip(right)
            .all(|(left, right)| (left - right).abs() <= tolerance)
    };
    let ExactProfileSegment::CircularArc {
        start_bits,
        end_bits,
        center_bits,
        clockwise,
    } = profile
        .segments
        .iter()
        .find(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?
    else {
        unreachable!("validated line-arc profile")
    };
    let arc_start = start_bits.map(f64::from_bits);
    let arc_end = end_bits.map(f64::from_bits);
    let center = center_bits.map(f64::from_bits);
    let start_outside = outside(arc_start);
    if start_outside == outside(arc_end) {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let radius = (arc_start[0] - center[0]).hypot(arc_start[1] - center[1]);
    let start_angle = (arc_start[1] - center[1]).atan2(arc_start[0] - center[0]);
    let end_angle = (arc_end[1] - center[1]).atan2(arc_end[0] - center[0]);
    let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let normalized_limit = (limit - center[axis]) / radius;
    if !normalized_limit.is_finite() || normalized_limit.abs() >= 1.0 - tolerance {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let principal = if axis == 0 {
        normalized_limit.acos()
    } else {
        normalized_limit.asin()
    };
    let angles = if axis == 0 {
        [principal, -principal]
    } else {
        [principal, std::f64::consts::PI - principal]
    };
    let mut contacts = angles.into_iter().filter_map(|angle| {
        if !angle_on_directed_arc(start_angle, sweep, angle) {
            return None;
        }
        let mut point = [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ];
        point[axis] = limit;
        (!same_point(point, arc_start) && !same_point(point, arc_end)).then_some((angle, point))
    });
    let (contact_angle, arc_contact) = contacts
        .next()
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    if contacts.next().is_some() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut crossing_lines = profile.segments.iter().filter_map(|segment| {
        let ExactProfileSegment::Line {
            start_bits,
            end_bits,
        } = segment
        else {
            return None;
        };
        let start = start_bits.map(f64::from_bits);
        let end = end_bits.map(f64::from_bits);
        (outside(start) != outside(end)).then_some((start, end))
    });
    let (line_start, line_end) = crossing_lines
        .next()
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    if crossing_lines.next().is_some() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let outside_endpoint = if start_outside { arc_start } else { arc_end };
    if (start_outside && !same_point(line_end, outside_endpoint))
        || (!start_outside && !same_point(line_start, outside_endpoint))
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let denominator = line_end[axis] - line_start[axis];
    if denominator.abs() <= tolerance {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let t = (limit - line_start[axis]) / denominator;
    if t <= tolerance || t >= 1.0 - tolerance {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut line_contact = [
        line_start[0] + t * (line_end[0] - line_start[0]),
        line_start[1] + t * (line_end[1] - line_start[1]),
    ];
    line_contact[axis] = limit;
    let (from_angle, to_angle) = if start_outside {
        (start_angle, contact_angle)
    } else {
        (contact_angle, end_angle)
    };
    let outside_sweep = directed_arc_sweep(from_angle, to_angle, *clockwise)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let steps = (outside_sweep.abs() / std::f64::consts::TAU * 64.0)
        .ceil()
        .max(2.0) as usize;
    let mut outside_path = (0..=steps)
        .map(|step| {
            let angle = from_angle + outside_sweep * step as f64 / steps as f64;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect::<Vec<_>>();
    if start_outside {
        outside_path.insert(0, line_contact);
        *outside_path.last_mut().expect("non-empty outside path") = arc_contact;
    } else {
        outside_path.push(line_contact);
        outside_path[0] = arc_contact;
    }
    outside_path.first_mut().expect("non-empty outside path")[axis] = limit;
    outside_path.last_mut().expect("non-empty outside path")[axis] = limit;
    let orthogonal_axis = 1 - axis;
    if (outside_path[0][orthogonal_axis] < outside_path[outside_path.len() - 1][orthogonal_axis])
        != ascending
    {
        outside_path.reverse();
    }
    let mut boundary = match (axis, limit == 0.0) {
        (0, false) => vec![[0.0, 0.0], [width, 0.0]],
        (1, false) => vec![[0.0, 0.0], [width, 0.0], [width, depth]],
        (0, true) => vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]],
        (1, true) => vec![[0.0, 0.0]],
        _ => unreachable!("validated clipping axis"),
    };
    boundary.extend(outside_path);
    match (axis, limit == 0.0) {
        (0, false) => boundary.extend([[width, depth], [0.0, depth]]),
        (1, false) => boundary.push([0.0, depth]),
        (0, true) => {}
        (1, true) => boundary.extend([[width, 0.0], [width, depth], [0.0, depth]]),
        _ => unreachable!("validated clipping axis"),
    }
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    render_profile_boundary_prism(&boundary, height, Some((center, radius)))
}

fn render_arc_only_clipped_side_overlap_union_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile
        .strict_convex_arc_only_clipped_side_overlap(width, depth)
        .is_none()
        && profile
            .d_profile_arc_only_clipped_side_overlap(width, depth)
            .is_none()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let ExactProfileSegment::CircularArc {
        start_bits,
        end_bits,
        center_bits,
        clockwise,
    } = profile
        .segments
        .iter()
        .find(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?
    else {
        unreachable!("validated arc-only profile")
    };
    let start = start_bits.map(f64::from_bits);
    let end = end_bits.map(f64::from_bits);
    let center = center_bits.map(f64::from_bits);
    let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
    let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
    let tolerance = 1.0e-6;
    let (axis, limit, extreme_angle, ascending) = if min_x < -tolerance {
        (0_usize, 0.0, std::f64::consts::PI, false)
    } else if max_x > width + tolerance {
        (0, width, 0.0, true)
    } else if min_y < -tolerance {
        (1, 0.0, 3.0 * std::f64::consts::FRAC_PI_2, true)
    } else if max_y > depth + tolerance {
        (1, depth, std::f64::consts::FRAC_PI_2, false)
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let distance = (center[axis] - limit).abs();
    let intersection_offset = (distance / radius).acos();
    let progress = |angle: f64| {
        if sweep > 0.0 {
            (angle - start_angle).rem_euclid(std::f64::consts::TAU)
        } else {
            (start_angle - angle).rem_euclid(std::f64::consts::TAU)
        }
    };
    let mut contacts = [
        extreme_angle - intersection_offset,
        extreme_angle + intersection_offset,
    ];
    contacts.sort_by(|left, right| progress(*left).total_cmp(&progress(*right)));
    let first_progress = progress(contacts[0]);
    let second_progress = progress(contacts[1]);
    if progress(extreme_angle) < first_progress - tolerance
        || progress(extreme_angle) > second_progress + tolerance
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let arc_sweep = second_progress - first_progress;
    let steps = (arc_sweep / std::f64::consts::TAU * 64.0).ceil().max(2.0) as usize;
    let direction = sweep.signum();
    let mut outside_arc = (0..=steps)
        .map(|step| {
            let angle = contacts[0] + direction * arc_sweep * step as f64 / steps as f64;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect::<Vec<_>>();
    outside_arc.first_mut().expect("non-empty arc")[axis] = limit;
    outside_arc.last_mut().expect("non-empty arc")[axis] = limit;
    let orthogonal_axis = 1 - axis;
    if (outside_arc[0][orthogonal_axis] < outside_arc[outside_arc.len() - 1][orthogonal_axis])
        != ascending
    {
        outside_arc.reverse();
    }
    let mut boundary = match (axis, limit == 0.0) {
        (0, false) => vec![[0.0, 0.0], [width, 0.0]],
        (1, false) => vec![[0.0, 0.0], [width, 0.0], [width, depth]],
        (0, true) => vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]],
        (1, true) => vec![[0.0, 0.0]],
        _ => unreachable!("validated clipping axis"),
    };
    boundary.extend(outside_arc);
    match (axis, limit == 0.0) {
        (0, false) => boundary.extend([[width, depth], [0.0, depth]]),
        (1, false) => boundary.push([0.0, depth]),
        (0, true) => {}
        (1, true) => boundary.extend([[width, 0.0], [width, depth], [0.0, depth]]),
        _ => unreachable!("validated clipping axis"),
    }
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    render_profile_boundary_prism(&boundary, height, Some((center, radius)))
}

fn mirror_mesh_across_vertical_axis(
    mut vertices: Vec<ExactVertex>,
    mut triangles: Vec<ExactTriangle>,
    width: f64,
) -> (Vec<ExactVertex>, Vec<ExactTriangle>) {
    for vertex in &mut vertices {
        vertex.position_mm[0] = width - vertex.position_mm[0];
    }
    for triangle in &mut triangles {
        triangle.vertex_indices.swap(1, 2);
    }
    (vertices, triangles)
}

fn mirror_mesh_across_horizontal_axis(
    mut vertices: Vec<ExactVertex>,
    mut triangles: Vec<ExactTriangle>,
    depth: f64,
) -> (Vec<ExactVertex>, Vec<ExactTriangle>) {
    for vertex in &mut vertices {
        vertex.position_mm[1] = depth - vertex.position_mm[1];
    }
    for triangle in &mut triangles {
        triangle.vertex_indices.swap(1, 2);
    }
    (vertices, triangles)
}

fn render_side_clipped_overlap_pocket_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
    pocket_depth: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile
        .strict_convex_line_arc_clipped_north_west_corner_overlap(width, depth)
        .is_some()
    {
        let mirrored = profile
            .mirrored_across_vertical_axis(width)
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let (vertices, triangles) =
            render_side_clipped_overlap_pocket_mesh(&mirrored, width, depth, height, pocket_depth)?;
        return Ok(mirror_mesh_across_vertical_axis(vertices, triangles, width));
    }
    if profile
        .strict_convex_line_arc_clipped_north_east_corner_overlap(width, depth)
        .is_some()
    {
        let mirrored = profile
            .mirrored_across_horizontal_axis(depth)
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let (vertices, triangles) =
            render_side_clipped_overlap_pocket_mesh(&mirrored, width, depth, height, pocket_depth)?;
        return Ok(mirror_mesh_across_horizontal_axis(
            vertices, triangles, depth,
        ));
    }
    if profile
        .strict_convex_line_arc_clipped_south_west_corner_overlap(width, depth)
        .is_some()
    {
        let mirrored = profile
            .mirrored_across_vertical_axis(width)
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let (vertices, triangles) =
            render_side_clipped_overlap_pocket_mesh(&mirrored, width, depth, height, pocket_depth)?;
        return Ok(mirror_mesh_across_vertical_axis(vertices, triangles, width));
    }
    if profile
        .rounded_rectangle_side_overlap_area(width, depth)
        .is_none()
        && profile
            .rounded_rectangle_corner_overlap_area(width, depth)
            .is_none()
        && profile
            .rounded_rectangle_arc_clipped_corner_overlap_area(width, depth)
            .is_none()
        && profile
            .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(width, depth)
            .is_none()
        && profile
            .d_profile_arc_only_clipped_side_overlap(width, depth)
            .is_none()
        && profile.capsule_side_overlap(width, depth).is_none()
        && profile.capsule_corner_overlap(width, depth).is_none()
        && profile
            .rounded_rectangle_chord_side_overlap(width, depth)
            .is_none()
        && profile
            .strict_convex_line_clipped_side_overlap(width, depth)
            .is_none()
        && profile
            .strict_convex_line_arc_clipped_side_overlap(width, depth)
            .is_none()
        && profile
            .strict_convex_arc_only_clipped_side_overlap(width, depth)
            .is_none()
        && profile
            .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
            .is_none()
        || !pocket_depth.is_finite()
        || pocket_depth <= 0.0
        || pocket_depth >= height
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let floor_z = height - pocket_depth;
    let corner_overlap = profile
        .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
        .is_some();
    let rounded_side_overlap = profile
        .rounded_rectangle_side_overlap_area(width, depth)
        .is_some();
    let rounded_corner_overlap = profile.capsule_corner_overlap(width, depth).is_some()
        || profile
            .rounded_rectangle_corner_overlap_area(width, depth)
            .is_some()
        || profile
            .rounded_rectangle_arc_clipped_corner_overlap_area(width, depth)
            .is_some()
        || profile
            .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(width, depth)
            .is_some();
    let (mut vertices, mut triangles) = if rounded_side_overlap {
        let [profile_min_x, profile_min_y, profile_max_x, profile_max_y] =
            profile.bounds_bits.map(f64::from_bits);
        let min_x = profile_min_x.max(0.0);
        let min_y = profile_min_y.max(0.0);
        let max_x = profile_max_x.min(width);
        let max_y = profile_max_y.min(depth);
        let tolerance = 1.0e-6;
        let (remaining_boundary, interface_axis, interface_coordinate) = if min_x <= tolerance
            && max_x < width - tolerance
            && min_y <= tolerance
            && max_y >= depth - tolerance
        {
            (
                vec![[max_x, 0.0], [width, 0.0], [width, depth], [max_x, depth]],
                0,
                max_x,
            )
        } else if min_x > tolerance
            && max_x >= width - tolerance
            && min_y <= tolerance
            && max_y >= depth - tolerance
        {
            (
                vec![[0.0, 0.0], [min_x, 0.0], [min_x, depth], [0.0, depth]],
                0,
                min_x,
            )
        } else if min_x <= tolerance
            && max_x >= width - tolerance
            && min_y <= tolerance
            && max_y < depth - tolerance
        {
            (
                vec![[0.0, max_y], [width, max_y], [width, depth], [0.0, depth]],
                1,
                max_y,
            )
        } else if min_x <= tolerance
            && max_x >= width - tolerance
            && min_y > tolerance
            && max_y >= depth - tolerance
        {
            (
                vec![[0.0, 0.0], [width, 0.0], [width, min_y], [0.0, min_y]],
                1,
                min_y,
            )
        } else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        let (remaining_vertices, mut remaining_triangles) =
            render_profile_boundary_prism(&remaining_boundary, height, None)?;
        for triangle in &mut remaining_triangles {
            if triangle.face_role.is_none()
                && triangle.vertex_indices.iter().all(|index| {
                    (remaining_vertices[*index as usize].position_mm[interface_axis]
                        - interface_coordinate)
                        .abs()
                        <= tolerance
                })
            {
                triangle.face_role = Some(ExactFaceRole::CutLinear);
            }
        }
        (remaining_vertices, remaining_triangles)
    } else if rounded_corner_overlap {
        render_corner_overlap_cut_mesh(profile, width, depth, height)?
    } else {
        render_line_clipped_side_overlap_cut_mesh(profile, width, depth, height)?
    };
    let mut floor_boundary = if rounded_corner_overlap {
        clipped_mixed_profile_boundary(profile, width, depth)?.0
    } else if corner_overlap {
        let outer_boundary = vertices
            .chunks_exact(2)
            .map(|pair| [pair[0].position_mm[0], pair[0].position_mm[1]])
            .collect::<Vec<_>>();
        let tolerance = 1.0e-6;
        let south_index = outer_boundary
            .iter()
            .position(|point| {
                point[1].abs() <= tolerance && point[0] > tolerance && point[0] < width - tolerance
            })
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let east_index = outer_boundary
            .iter()
            .position(|point| {
                (point[0] - width).abs() <= tolerance
                    && point[1] > tolerance
                    && point[1] < depth - tolerance
            })
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let mut boundary = vec![outer_boundary[south_index]];
        let mut index = (south_index + 1) % outer_boundary.len();
        while index != east_index {
            boundary.push(outer_boundary[index]);
            index = (index + 1) % outer_boundary.len();
            if index == south_index {
                return Err(ExactProductError::InvalidWorkerEvidence);
            }
        }
        boundary.extend([outer_boundary[east_index], [width, 0.0]]);
        boundary
    } else if rounded_side_overlap {
        let [profile_min_x, profile_min_y, profile_max_x, profile_max_y] =
            profile.bounds_bits.map(f64::from_bits);
        vec![
            [profile_min_x.max(0.0), profile_min_y.max(0.0)],
            [profile_max_x.min(width), profile_min_y.max(0.0)],
            [profile_max_x.min(width), profile_max_y.min(depth)],
            [profile_min_x.max(0.0), profile_max_y.min(depth)],
        ]
    } else {
        clipped_mixed_profile_boundary(profile, width, depth)?.0
    };
    if polygon_signed_area(&floor_boundary) < 0.0 {
        floor_boundary.reverse();
    }
    for vertex in &mut vertices {
        if vertex.position_mm[2] == 0.0 {
            vertex.position_mm[2] = floor_z;
        }
    }
    triangles.retain(|triangle| triangle.face_role != Some(ExactFaceRole::Bottom));
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::ArcSide) {
            triangle.face_role = Some(ExactFaceRole::CutLinear);
        }
    }

    let tolerance = 1.0e-6;
    let perimeter_position = |point: [f64; 2]| {
        if point[1].abs() <= tolerance {
            Some(point[0])
        } else if (point[0] - width).abs() <= tolerance {
            Some(width + point[1])
        } else if (point[1] - depth).abs() <= tolerance {
            Some(2.0 * width + depth - point[0])
        } else if point[0].abs() <= tolerance {
            Some(2.0 * (width + depth) - point[1])
        } else {
            None
        }
    };
    let mut positioned_boundary = vec![
        (0.0, [0.0, 0.0]),
        (width, [width, 0.0]),
        (width + depth, [width, depth]),
        (2.0 * width + depth, [0.0, depth]),
    ];
    for point in floor_boundary.iter().copied() {
        if let Some(position) = perimeter_position(point)
            && !positioned_boundary
                .iter()
                .any(|(existing, _)| (existing - position).abs() <= tolerance)
        {
            positioned_boundary.push((position, point));
        }
    }
    positioned_boundary.sort_by(|left, right| left.0.total_cmp(&right.0));
    let lower_boundary = positioned_boundary
        .into_iter()
        .map(|(_, point)| point)
        .collect::<Vec<_>>();
    let (lower_vertices, mut lower_triangles) =
        render_profile_boundary_prism(&lower_boundary, floor_z, None)?;
    lower_triangles.retain(|triangle| triangle.face_role != Some(ExactFaceRole::Top));
    for triangle in &mut lower_triangles {
        if triangle
            .vertex_indices
            .iter()
            .all(|index| (lower_vertices[*index as usize].position_mm[0] - width).abs() <= 1.0e-9)
        {
            triangle.face_role = Some(ExactFaceRole::East);
        }
        triangle.vertex_indices = triangle
            .vertex_indices
            .map(|index| index + vertices.len() as u32);
    }
    vertices.extend(lower_vertices);
    triangles.extend(lower_triangles);

    let floor_start = vertices.len() as u32;
    vertices.extend(floor_boundary.iter().map(|point| ExactVertex {
        position_mm: [point[0], point[1], floor_z],
    }));
    for [a, b, c] in triangulate_polygon(&floor_boundary)? {
        triangles.push(ExactTriangle {
            vertex_indices: [floor_start + a, floor_start + b, floor_start + c],
            face_role: Some(ExactFaceRole::PocketFloor),
        });
    }

    let weld_tolerance = width.min(depth) * 1.0e-9;
    let mut welded_vertices: Vec<ExactVertex> = Vec::new();
    let remap = vertices
        .into_iter()
        .map(|vertex| {
            welded_vertices
                .iter()
                .position(|candidate| {
                    candidate
                        .position_mm
                        .into_iter()
                        .zip(vertex.position_mm)
                        .all(|(left, right)| (left - right).abs() <= weld_tolerance)
                })
                .map_or_else(
                    || {
                        welded_vertices.push(vertex);
                        (welded_vertices.len() - 1) as u32
                    },
                    |index| index as u32,
                )
        })
        .collect::<Vec<_>>();
    let welded_triangles = triangles
        .into_iter()
        .filter_map(|mut triangle| {
            triangle.vertex_indices = triangle.vertex_indices.map(|index| remap[index as usize]);
            let [a, b, c] = triangle.vertex_indices;
            (a != b && b != c && c != a).then_some(triangle)
        })
        .collect();
    Ok((welded_vertices, welded_triangles))
}

fn render_line_clipped_side_overlap_cut_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile
        .strict_convex_line_arc_clipped_north_west_corner_overlap(width, depth)
        .is_some()
    {
        let mirrored = profile
            .mirrored_across_vertical_axis(width)
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let (vertices, triangles) =
            render_line_clipped_side_overlap_cut_mesh(&mirrored, width, depth, height)?;
        return Ok(mirror_mesh_across_vertical_axis(vertices, triangles, width));
    }
    if profile
        .strict_convex_line_arc_clipped_north_east_corner_overlap(width, depth)
        .is_some()
    {
        let mirrored = profile
            .mirrored_across_horizontal_axis(depth)
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let (vertices, triangles) =
            render_line_clipped_side_overlap_cut_mesh(&mirrored, width, depth, height)?;
        return Ok(mirror_mesh_across_horizontal_axis(
            vertices, triangles, depth,
        ));
    }
    if profile
        .strict_convex_line_arc_clipped_south_west_corner_overlap(width, depth)
        .is_some()
    {
        let mirrored = profile
            .mirrored_across_vertical_axis(width)
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let (vertices, triangles) =
            render_line_clipped_side_overlap_cut_mesh(&mirrored, width, depth, height)?;
        return Ok(mirror_mesh_across_vertical_axis(vertices, triangles, width));
    }
    if profile
        .strict_convex_line_clipped_side_overlap(width, depth)
        .is_none()
        && profile
            .strict_convex_arc_only_clipped_side_overlap(width, depth)
            .is_none()
        && profile
            .d_profile_arc_only_clipped_side_overlap(width, depth)
            .is_none()
        && profile
            .strict_convex_line_arc_clipped_side_overlap(width, depth)
            .is_none()
        && profile.capsule_side_overlap(width, depth).is_none()
        && profile
            .rounded_rectangle_chord_side_overlap(width, depth)
            .is_none()
        && profile
            .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
            .is_none()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let (vertices, triangles) = if profile
        .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
        .is_some()
    {
        return render_south_east_corner_overlap_cut_mesh(profile, width, depth, height);
    } else {
        render_line_clipped_side_overlap_split_mesh(profile, width, depth, height)?
    };
    let seed = vertices
        .iter()
        .position(|vertex| vertex.position_mm == [0.0, 0.0, 0.0])
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let mut reachable_vertices = vec![false; vertices.len()];
    let mut reachable_triangles = vec![false; triangles.len()];
    reachable_vertices[seed] = true;
    loop {
        let mut changed = false;
        for (index, triangle) in triangles.iter().enumerate() {
            if !reachable_triangles[index]
                && triangle
                    .vertex_indices
                    .iter()
                    .any(|vertex| reachable_vertices[*vertex as usize])
            {
                reachable_triangles[index] = true;
                changed = true;
                for vertex in triangle.vertex_indices {
                    reachable_vertices[vertex as usize] = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    if reachable_triangles.iter().all(|reachable| *reachable)
        || reachable_triangles.iter().all(|reachable| !*reachable)
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut remap = vec![u32::MAX; vertices.len()];
    let mut cut_vertices = Vec::new();
    for (index, vertex) in vertices.into_iter().enumerate() {
        if reachable_vertices[index] {
            remap[index] = cut_vertices.len() as u32;
            cut_vertices.push(vertex);
        }
    }
    let cut_triangles = triangles
        .into_iter()
        .zip(reachable_triangles)
        .filter(|(_, reachable)| *reachable)
        .map(|(mut triangle, _)| {
            triangle.vertex_indices = triangle.vertex_indices.map(|vertex| remap[vertex as usize]);
            if (profile.capsule_side_overlap(width, depth).is_some()
                || profile
                    .rounded_rectangle_chord_side_overlap(width, depth)
                    .is_some())
                && triangle.face_role == Some(ExactFaceRole::ArcSide)
            {
                triangle.face_role = Some(ExactFaceRole::CutLinear);
            }
            triangle
        })
        .collect();
    Ok((cut_vertices, cut_triangles))
}

fn render_line_clipped_side_overlap_split_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile
        .strict_convex_line_clipped_side_overlap(width, depth)
        .is_none()
        && profile
            .strict_convex_arc_only_clipped_side_overlap(width, depth)
            .is_none()
        && profile
            .d_profile_arc_only_clipped_side_overlap(width, depth)
            .is_none()
        && profile
            .strict_convex_line_arc_clipped_side_overlap(width, depth)
            .is_none()
        && profile.capsule_side_overlap(width, depth).is_none()
        && profile
            .rounded_rectangle_chord_side_overlap(width, depth)
            .is_none()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let (mut vertices, mut triangles) =
        render_clipped_mixed_profile_mesh(profile, width, depth, height)?;
    let (mut inner_boundary, arc_center, arc_radius) =
        clipped_mixed_profile_boundary(profile, width, depth)?;
    if polygon_signed_area(&inner_boundary) < 0.0 {
        inner_boundary.reverse();
    }

    let tolerance = 1.0e-6;
    let mut clip_edges = inner_boundary
        .iter()
        .enumerate()
        .filter_map(|(index, start)| {
            let end = inner_boundary[(index + 1) % inner_boundary.len()];
            let on_same_side = ((start[0].abs() <= tolerance && end[0].abs() <= tolerance)
                || ((start[0] - width).abs() <= tolerance && (end[0] - width).abs() <= tolerance)
                || (start[1].abs() <= tolerance && end[1].abs() <= tolerance)
                || ((start[1] - depth).abs() <= tolerance && (end[1] - depth).abs() <= tolerance))
                && (start[0] - end[0]).hypot(start[1] - end[1]) > tolerance;
            on_same_side.then_some(index)
        });
    let clip_edge = clip_edges
        .next()
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    if clip_edges.next().is_some() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let clip_start = inner_boundary[clip_edge];
    let clip_end = inner_boundary[(clip_edge + 1) % inner_boundary.len()];
    let perimeter = 2.0 * (width + depth);
    let perimeter_position = |point: [f64; 2]| {
        if point[1].abs() <= tolerance {
            Some(point[0])
        } else if (point[0] - width).abs() <= tolerance {
            Some(width + point[1])
        } else if (point[1] - depth).abs() <= tolerance {
            Some(width + depth + width - point[0])
        } else if point[0].abs() <= tolerance {
            Some(2.0 * width + depth + depth - point[1])
        } else {
            None
        }
    };
    let path_start =
        perimeter_position(clip_end).ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let mut path_end =
        perimeter_position(clip_start).ok_or(ExactProductError::InvalidWorkerEvidence)?;
    while path_end <= path_start + tolerance {
        path_end += perimeter;
    }
    if path_end - path_start <= perimeter * 0.5 {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }

    let corners = [
        (0.0, [0.0, 0.0]),
        (width, [width, 0.0]),
        (width + depth, [width, depth]),
        (2.0 * width + depth, [0.0, depth]),
    ];
    let mut outer_boundary = vec![clip_end];
    for cycle in 0..=1 {
        for (position, corner) in corners {
            let position = position + f64::from(cycle) * perimeter;
            if position > path_start + tolerance && position < path_end - tolerance {
                outer_boundary.push(corner);
            }
        }
    }
    outer_boundary.push(clip_start);
    let interface_end = (clip_edge + 1) % inner_boundary.len();
    let mut index = if clip_edge == 0 {
        inner_boundary.len() - 1
    } else {
        clip_edge - 1
    };
    while index != interface_end {
        outer_boundary.push(inner_boundary[index]);
        index = if index == 0 {
            inner_boundary.len() - 1
        } else {
            index - 1
        };
    }
    if outer_boundary.len() < 4
        || polygon_signed_area(&outer_boundary).abs() <= 1.0e-12
        || !outer_boundary
            .iter()
            .all(|point| point.iter().all(|value| value.is_finite()))
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    if polygon_signed_area(&outer_boundary) < 0.0 {
        outer_boundary.reverse();
    }
    let (outer_vertices, mut outer_triangles) =
        render_profile_boundary_prism(&outer_boundary, height, Some((arc_center, arc_radius)))?;
    let offset = vertices.len() as u32;
    for triangle in &mut outer_triangles {
        triangle.vertex_indices = triangle.vertex_indices.map(|index| index + offset);
    }
    vertices.extend(outer_vertices);
    triangles.extend(outer_triangles);
    Ok((vertices, triangles))
}

fn render_south_east_corner_overlap_cut_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile
        .strict_convex_line_arc_clipped_south_east_corner_overlap(width, depth)
        .is_none()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let tolerance = 1.0e-6;
    let (arc_index, arc_start, arc_end, arc_center, clockwise) = profile
        .segments
        .iter()
        .enumerate()
        .find_map(|(index, segment)| {
            let ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } = segment
            else {
                return None;
            };
            Some((
                index,
                start_bits.map(f64::from_bits),
                end_bits.map(f64::from_bits),
                center_bits.map(f64::from_bits),
                *clockwise,
            ))
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let arc_radius = (arc_start[0] - arc_center[0]).hypot(arc_start[1] - arc_center[1]);
    let start_angle = (arc_start[1] - arc_center[1]).atan2(arc_start[0] - arc_center[0]);
    let end_angle = (arc_end[1] - arc_center[1]).atan2(arc_end[0] - arc_center[0]);
    let sweep = directed_arc_sweep(start_angle, end_angle, clockwise)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let principal = ((width - arc_center[0]) / arc_radius).acos();
    let mut east_angles = [principal, -principal].into_iter().filter(|angle| {
        angle_on_directed_arc(start_angle, sweep, *angle)
            && arc_center[1] + arc_radius * angle.sin() > tolerance
    });
    let east_angle = east_angles
        .next()
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    if east_angles.next().is_some() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let east_contact = [width, arc_center[1] + arc_radius * east_angle.sin()];

    let (crossing_index, crossing_start, crossing_end) = profile
        .segments
        .iter()
        .enumerate()
        .find_map(|(index, segment)| {
            let ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } = segment
            else {
                return None;
            };
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            (start[1] > tolerance && end[1] < -tolerance).then_some((index, start, end))
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let t = -crossing_start[1] / (crossing_end[1] - crossing_start[1]);
    let south_contact = [
        crossing_start[0] + t * (crossing_end[0] - crossing_start[0]),
        0.0,
    ];
    let mut outer_boundary = vec![east_contact];
    outer_boundary.extend([[width, depth], [0.0, depth], [0.0, 0.0], south_contact]);
    outer_boundary.push(crossing_start);
    let mut index = if crossing_index == 0 {
        profile.segments.len() - 1
    } else {
        crossing_index - 1
    };
    while index != arc_index {
        let ExactProfileSegment::Line { start_bits, .. } = &profile.segments[index] else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        outer_boundary.push(start_bits.map(f64::from_bits));
        index = if index == 0 {
            profile.segments.len() - 1
        } else {
            index - 1
        };
    }
    let reverse_sweep = directed_arc_sweep(end_angle, east_angle, !clockwise)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let steps = (reverse_sweep.abs() / std::f64::consts::TAU * 64.0)
        .ceil()
        .max(1.0) as usize;
    for step in 1..steps {
        let angle = end_angle + reverse_sweep * step as f64 / steps as f64;
        outer_boundary.push([
            arc_center[0] + arc_radius * angle.cos(),
            arc_center[1] + arc_radius * angle.sin(),
        ]);
    }
    if outer_boundary.len() < 4
        || polygon_signed_area(&outer_boundary).abs() <= 1.0e-12
        || !outer_boundary
            .iter()
            .all(|point| point.iter().all(|value| value.is_finite()))
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    if polygon_signed_area(&outer_boundary) < 0.0 {
        outer_boundary.reverse();
    }
    render_profile_boundary_prism(&outer_boundary, height, Some((arc_center, arc_radius)))
}

fn render_corner_overlap_union_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let (inner_boundary, arc_center, arc_radius) =
        clipped_mixed_profile_boundary(profile, width, depth)?;
    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
    let tolerance = 1.0e-6;
    let corner_x = if min_x > tolerance && max_x > width + tolerance {
        width
    } else if min_x < -tolerance && max_x < width - tolerance {
        0.0
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let corner_y = if min_y > tolerance && max_y > depth + tolerance {
        depth
    } else if min_y < -tolerance && max_y < depth - tolerance {
        0.0
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let x_contact = inner_boundary
        .iter()
        .copied()
        .find(|point| {
            (point[0] - corner_x).abs() <= tolerance && (point[1] - corner_y).abs() > tolerance
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let y_contact = inner_boundary
        .iter()
        .copied()
        .find(|point| {
            (point[1] - corner_y).abs() <= tolerance && (point[0] - corner_x).abs() > tolerance
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;

    let (profile_vertices, _) = render_mixed_profile_mesh(profile, height)?;
    let mut profile_boundary = profile_vertices
        .chunks_exact(2)
        .map(|pair| {
            let [x, y, _] = pair[0].position_mm;
            [x, y]
        })
        .collect::<Vec<_>>();
    let insert_contact =
        |boundary: &mut Vec<[f64; 2]>, contact: [f64; 2]| -> Result<usize, ExactProductError> {
            if let Some(index) = boundary.iter().position(|point| {
                point
                    .iter()
                    .zip(contact)
                    .all(|(actual, expected)| (*actual - expected).abs() <= tolerance)
            }) {
                return Ok(index);
            }
            let edge = (0..boundary.len()).find(|index| {
                let start = boundary[*index];
                let end = boundary[(*index + 1) % boundary.len()];
                let delta = [end[0] - start[0], end[1] - start[1]];
                let length_squared = delta[0] * delta[0] + delta[1] * delta[1];
                if length_squared <= tolerance * tolerance {
                    return false;
                }
                let parameter = ((contact[0] - start[0]) * delta[0]
                    + (contact[1] - start[1]) * delta[1])
                    / length_squared;
                if parameter <= 0.0 || parameter >= 1.0 {
                    return false;
                }
                let projected = [
                    start[0] + parameter * delta[0],
                    start[1] + parameter * delta[1],
                ];
                projected
                    .iter()
                    .zip(contact)
                    .all(|(actual, expected)| (*actual - expected).abs() <= tolerance)
            });
            let edge = edge.ok_or(ExactProductError::InvalidWorkerEvidence)?;
            let index = edge + 1;
            boundary.insert(index, contact);
            Ok(index)
        };
    insert_contact(&mut profile_boundary, x_contact)?;
    insert_contact(&mut profile_boundary, y_contact)?;
    let x_index = insert_contact(&mut profile_boundary, x_contact)?;
    let y_index = insert_contact(&mut profile_boundary, y_contact)?;
    let cyclic_path = |start: usize, end: usize| {
        let mut path = Vec::new();
        let mut index = start;
        loop {
            path.push(profile_boundary[index]);
            if index == end {
                break;
            }
            index = (index + 1) % profile_boundary.len();
        }
        path
    };
    let forward = cyclic_path(y_index, x_index);
    let backward = {
        let mut path = cyclic_path(x_index, y_index);
        path.reverse();
        path
    };
    let outside_count = |path: &[[f64; 2]]| {
        path.iter()
            .filter(|point| {
                point[0] < -tolerance
                    || point[0] > width + tolerance
                    || point[1] < -tolerance
                    || point[1] > depth + tolerance
            })
            .count()
    };
    let forward_outside = outside_count(&forward);
    let backward_outside = outside_count(&backward);
    let outside_path = if forward_outside > backward_outside {
        forward
    } else if backward_outside > forward_outside {
        backward
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    if outside_count(&outside_path) == 0 {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }

    let mut outer_boundary = vec![x_contact];
    match (corner_x == width, corner_y == depth) {
        (true, true) => outer_boundary.extend([[width, 0.0], [0.0, 0.0], [0.0, depth]]),
        (false, true) => outer_boundary.extend([[0.0, 0.0], [width, 0.0], [width, depth]]),
        (true, false) => outer_boundary.extend([[width, depth], [0.0, depth], [0.0, 0.0]]),
        (false, false) => outer_boundary.extend([[0.0, depth], [width, depth], [width, 0.0]]),
    }
    outer_boundary.push(y_contact);
    let outside_interior_len = outside_path.len().saturating_sub(2);
    outer_boundary.extend(outside_path.into_iter().skip(1).take(outside_interior_len));
    outer_boundary.dedup_by(|left, right| {
        left.iter()
            .zip(right.iter())
            .all(|(left, right)| (*left - *right).abs() <= tolerance)
    });
    if outer_boundary.len() < 4
        || polygon_signed_area(&outer_boundary).abs() <= 1.0e-12
        || !outer_boundary
            .iter()
            .all(|point| point.iter().all(|value| value.is_finite()))
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    if polygon_signed_area(&outer_boundary) < 0.0 {
        outer_boundary.reverse();
    }
    render_profile_boundary_prism(&outer_boundary, height, Some((arc_center, arc_radius)))
}

fn render_corner_overlap_split_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let (mut vertices, mut triangles) =
        render_clipped_mixed_profile_mesh(profile, width, depth, height)?;
    let (inner_boundary, arc_center, arc_radius) =
        clipped_mixed_profile_boundary(profile, width, depth)?;
    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
    let tolerance = 1.0e-6;
    let corner_x = if min_x > tolerance && max_x > width + tolerance {
        width
    } else if min_x < -tolerance && max_x < width - tolerance {
        0.0
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let corner_y = if min_y > tolerance && max_y > depth + tolerance {
        depth
    } else if min_y < -tolerance && max_y < depth - tolerance {
        0.0
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let same_point = |left: [f64; 2], right: [f64; 2]| {
        left.into_iter()
            .zip(right)
            .all(|(left, right)| (left - right).abs() <= tolerance)
    };
    let corner = [corner_x, corner_y];
    let x_contact = inner_boundary
        .iter()
        .position(|point| {
            (point[0] - corner_x).abs() <= tolerance && (point[1] - corner_y).abs() > tolerance
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let y_contact = inner_boundary
        .iter()
        .position(|point| {
            (point[1] - corner_y).abs() <= tolerance && (point[0] - corner_x).abs() > tolerance
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let cyclic_path = |start: usize, end: usize| {
        let mut path = Vec::new();
        let mut index = start;
        loop {
            path.push(inner_boundary[index]);
            if index == end {
                break;
            }
            index = (index + 1) % inner_boundary.len();
        }
        path
    };
    let first_path = cyclic_path(y_contact, x_contact);
    let second_path = {
        let mut path = cyclic_path(x_contact, y_contact);
        path.reverse();
        path
    };
    let interface = [first_path, second_path]
        .into_iter()
        .find(|path| !path.iter().any(|point| same_point(*point, corner)))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let mut outer_boundary = vec![inner_boundary[x_contact]];
    match (corner_x == width, corner_y == depth) {
        (true, true) => outer_boundary.extend([[width, 0.0], [0.0, 0.0], [0.0, depth]]),
        (false, true) => outer_boundary.extend([[0.0, 0.0], [width, 0.0], [width, depth]]),
        (true, false) => outer_boundary.extend([[width, depth], [0.0, depth], [0.0, 0.0]]),
        (false, false) => outer_boundary.extend([[0.0, depth], [width, depth], [width, 0.0]]),
    }
    outer_boundary.push(inner_boundary[y_contact]);
    outer_boundary.extend(
        interface
            .iter()
            .copied()
            .skip(1)
            .take(interface.len().saturating_sub(2)),
    );
    if outer_boundary.len() < 4
        || polygon_signed_area(&outer_boundary).abs() <= 1.0e-12
        || !outer_boundary
            .iter()
            .all(|point| point.iter().all(|value| value.is_finite()))
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    if polygon_signed_area(&outer_boundary) < 0.0 {
        outer_boundary.reverse();
    }
    let (outer_vertices, mut outer_triangles) =
        render_profile_boundary_prism(&outer_boundary, height, Some((arc_center, arc_radius)))?;
    let offset = vertices.len() as u32;
    for triangle in &mut outer_triangles {
        triangle.vertex_indices = triangle.vertex_indices.map(|index| index + offset);
    }
    vertices.extend(outer_vertices);
    triangles.extend(outer_triangles);
    Ok((vertices, triangles))
}

fn render_corner_overlap_cut_mesh(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if profile.capsule_corner_overlap(width, depth).is_none()
        && profile
            .rounded_rectangle_corner_overlap_area(width, depth)
            .is_none()
        && profile
            .rounded_rectangle_arc_clipped_corner_overlap_area(width, depth)
            .is_none()
        && profile
            .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(width, depth)
            .is_none()
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let curved_cut_role = if profile
        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(width, depth)
        .is_some()
    {
        ExactFaceRole::CutArc
    } else {
        ExactFaceRole::CutLinear
    };
    let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
    let tolerance = 1.0e-6;
    let seed_position = if min_x > tolerance && min_y > tolerance {
        [0.0, 0.0, 0.0]
    } else if max_x < width - tolerance && min_y > tolerance {
        [width, 0.0, 0.0]
    } else if min_x > tolerance && max_y < depth - tolerance {
        [0.0, depth, 0.0]
    } else if max_x < width - tolerance && max_y < depth - tolerance {
        [width, depth, 0.0]
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let (vertices, triangles) = render_corner_overlap_split_mesh(profile, width, depth, height)?;
    let seed = vertices
        .iter()
        .position(|vertex| vertex.position_mm == seed_position)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let mut reachable_vertices = vec![false; vertices.len()];
    let mut reachable_triangles = vec![false; triangles.len()];
    reachable_vertices[seed] = true;
    loop {
        let mut changed = false;
        for (index, triangle) in triangles.iter().enumerate() {
            if !reachable_triangles[index]
                && triangle
                    .vertex_indices
                    .iter()
                    .any(|vertex| reachable_vertices[*vertex as usize])
            {
                reachable_triangles[index] = true;
                changed = true;
                for vertex in triangle.vertex_indices {
                    reachable_vertices[vertex as usize] = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    if reachable_triangles.iter().all(|reachable| *reachable)
        || reachable_triangles.iter().all(|reachable| !*reachable)
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut remap = vec![u32::MAX; vertices.len()];
    let mut cut_vertices = Vec::new();
    for (index, vertex) in vertices.into_iter().enumerate() {
        if reachable_vertices[index] {
            remap[index] = cut_vertices.len() as u32;
            cut_vertices.push(vertex);
        }
    }
    let cut_triangles = triangles
        .into_iter()
        .zip(reachable_triangles)
        .filter(|(_, reachable)| *reachable)
        .map(|(mut triangle, _)| {
            triangle.vertex_indices = triangle.vertex_indices.map(|vertex| remap[vertex as usize]);
            if triangle.face_role == Some(ExactFaceRole::ArcSide) {
                triangle.face_role = Some(curved_cut_role);
            }
            triangle
        })
        .collect();
    Ok((cut_vertices, cut_triangles))
}

fn clipped_mixed_profile_boundary(
    profile: &ExactMixedProfile,
    width: f64,
    depth: f64,
) -> Result<ClippedProfileBoundary, ExactProductError> {
    let mut boundary = Vec::<[f64; 2]>::new();
    for (segment_index, segment) in profile.segments.iter().enumerate() {
        let (start, end, center, sweep, steps) = match segment {
            ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } => (
                start_bits.map(f64::from_bits),
                end_bits.map(f64::from_bits),
                [0.0; 2],
                0.0,
                1,
            ),
            ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => {
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)
                    .ok_or(ExactProductError::InvalidWorkerEvidence)?;
                let steps = (sweep.abs() / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
                (start, end, center, sweep, steps)
            }
        };
        if boundary.is_empty() {
            boundary.push(start);
        } else if boundary.last().is_none_or(|previous| {
            previous
                .iter()
                .zip(start)
                .any(|(actual, expected)| (actual - expected).abs() > 1.0e-9)
        }) {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        for step in 1..=steps {
            let point = if sweep == 0.0 || step == steps {
                end
            } else {
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let angle = start_angle + sweep * step as f64 / steps as f64;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            };
            let closes_boundary = segment_index + 1 == profile.segments.len()
                && step == steps
                && point
                    .into_iter()
                    .zip(boundary[0])
                    .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-9);
            if !closes_boundary {
                boundary.push(point);
            }
        }
    }
    for (axis, limit, keep_greater) in [
        (0, 0.0, true),
        (0, width, false),
        (1, 0.0, true),
        (1, depth, false),
    ] {
        boundary = clip_polygon_to_half_plane(boundary, axis, limit, keep_greater);
    }
    boundary.dedup_by(|left, right| {
        left.iter()
            .zip(right.iter())
            .all(|(left, right)| (*left - *right).abs() <= 1.0e-9)
    });
    if boundary.len() >= 2
        && boundary[0]
            .into_iter()
            .zip(*boundary.last().expect("non-empty clipped boundary"))
            .all(|(left, right)| (left - right).abs() <= 1.0e-9)
    {
        boundary.pop();
    }
    let (arc_center, arc_radius) = profile
        .segments
        .iter()
        .filter_map(|segment| match segment {
            ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                Some((center, (start[0] - center[0]).hypot(start[1] - center[1])))
            }
            ExactProfileSegment::Line { .. } => None,
        })
        .find(|(center, radius)| {
            let tolerance = radius.max(1.0) * 1.0e-6;
            boundary.iter().enumerate().any(|(index, start)| {
                let end = boundary[(index + 1) % boundary.len()];
                [*start, end].into_iter().all(|point| {
                    ((point[0] - center[0]).hypot(point[1] - center[1]) - radius).abs() <= tolerance
                })
            })
        })
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    Ok((boundary, arc_center, arc_radius))
}

fn render_profile_boundary_prism(
    boundary: &[[f64; 2]],
    height: f64,
    arc: Option<([f64; 2], f64)>,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let cap_triangles = triangulate_polygon(boundary)?;
    let mut vertices = Vec::with_capacity(boundary.len() * 2);
    for point in boundary {
        vertices.extend([
            ExactVertex {
                position_mm: [point[0], point[1], 0.0],
            },
            ExactVertex {
                position_mm: [point[0], point[1], height],
            },
        ]);
    }
    let signed_area = polygon_signed_area(boundary);
    let mut triangles = Vec::with_capacity(cap_triangles.len() * 2 + boundary.len() * 2);
    for [a, b, c] in cap_triangles {
        let top = if signed_area > 0.0 {
            [a, b, c]
        } else {
            [a, c, b]
        };
        triangles.extend([
            ExactTriangle {
                vertex_indices: [top[0] * 2 + 1, top[1] * 2 + 1, top[2] * 2 + 1],
                face_role: Some(ExactFaceRole::Top),
            },
            ExactTriangle {
                vertex_indices: [top[0] * 2, top[2] * 2, top[1] * 2],
                face_role: Some(ExactFaceRole::Bottom),
            },
        ]);
    }
    for (index, start) in boundary.iter().enumerate() {
        let next = (index + 1) % boundary.len();
        let end = boundary[next];
        let role = arc.and_then(|(center, radius)| {
            let tolerance = radius.max(1.0) * 1.0e-6;
            let on_arc = |point: [f64; 2]| {
                ((point[0] - center[0]).hypot(point[1] - center[1]) - radius).abs() <= tolerance
            };
            (on_arc(*start) && on_arc(end)).then_some(ExactFaceRole::ArcSide)
        });
        let bottom = (index * 2) as u32;
        let top = bottom + 1;
        let next_bottom = (next * 2) as u32;
        let next_top = next_bottom + 1;
        triangles.extend([
            ExactTriangle {
                vertex_indices: [bottom, next_bottom, next_top],
                face_role: role,
            },
            ExactTriangle {
                vertex_indices: [bottom, next_top, top],
                face_role: role,
            },
        ]);
    }
    Ok((vertices, triangles))
}

fn circle_contains_rectangle(circle: ExactCircleProfile, width: f64, depth: f64) -> bool {
    let center = [
        f64::from_bits(circle.center_x_bits),
        f64::from_bits(circle.center_y_bits),
    ];
    let radius = f64::from_bits(circle.radius_bits);
    [[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]]
        .into_iter()
        .all(|corner| (corner[0] - center[0]).hypot(corner[1] - center[1]) < radius - 1.0e-6)
}

fn polygon_within_rectangle(profile: &ExactMixedProfile, width: f64, depth: f64) -> bool {
    let bounds = profile.bounds_bits.map(f64::from_bits);
    bounds[0] > 1.0e-6
        && bounds[1] > 1.0e-6
        && bounds[2] < width - 1.0e-6
        && bounds[3] < depth - 1.0e-6
}

fn polygon_contains_rectangle(profile: &ExactMixedProfile, width: f64, depth: f64) -> bool {
    let mut polygon = Vec::new();
    for segment in &profile.segments {
        match segment {
            ExactProfileSegment::Line { start_bits, .. } => {
                polygon.push(start_bits.map(f64::from_bits));
            }
            ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => {
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                let Some(sweep) = directed_arc_sweep(start_angle, end_angle, *clockwise) else {
                    return false;
                };
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let steps = (sweep.abs() / std::f64::consts::TAU * 256.0)
                    .ceil()
                    .max(1.0) as usize;
                for step in 0..steps {
                    let angle = start_angle + sweep * step as f64 / steps as f64;
                    polygon.push([
                        center[0] + radius * angle.cos(),
                        center[1] + radius * angle.sin(),
                    ]);
                }
            }
        }
    }
    if polygon.len() < 3 {
        return false;
    }
    [[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]]
        .into_iter()
        .all(|point| point_strictly_in_polygon(point, &polygon))
}

fn point_strictly_in_polygon(point: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let near_boundary = polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .any(|(start, end)| {
            let edge_length = (end[0] - start[0]).hypot(end[1] - start[1]);
            triangle_cross(*start, *end, point).abs() <= 1.0e-6 * edge_length
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
        let cross = triangle_cross(*start, *end, point);
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

fn polygon_signed_area(points: &[[f64; 2]]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        * 0.5
}

fn triangulate_polygon(points: &[[f64; 2]]) -> Result<Vec<[u32; 3]>, ExactProductError> {
    let orientation = polygon_signed_area(points).signum();
    if orientation == 0.0 {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut remaining = (0..points.len()).collect::<Vec<_>>();
    let mut triangles = Vec::with_capacity(points.len() - 2);
    while remaining.len() > 3 {
        let mut ear = None;
        for index in 0..remaining.len() {
            let previous = remaining[(index + remaining.len() - 1) % remaining.len()];
            let current = remaining[index];
            let next = remaining[(index + 1) % remaining.len()];
            let cross = triangle_cross(points[previous], points[current], points[next]);
            if cross * orientation <= 1.0e-12 {
                continue;
            }
            if remaining.iter().copied().any(|candidate| {
                candidate != previous
                    && candidate != current
                    && candidate != next
                    && point_in_triangle(
                        points[candidate],
                        points[previous],
                        points[current],
                        points[next],
                    )
            }) {
                continue;
            }
            ear = Some((index, [previous as u32, current as u32, next as u32]));
            break;
        }
        let Some((index, triangle)) = ear else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        triangles.push(triangle);
        remaining.remove(index);
    }
    triangles.push([
        remaining[0] as u32,
        remaining[1] as u32,
        remaining[2] as u32,
    ]);
    Ok(triangles)
}

fn triangle_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_triangle(point: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let crosses = [
        triangle_cross(a, b, point),
        triangle_cross(b, c, point),
        triangle_cross(c, a, point),
    ];
    let has_negative = crosses.iter().any(|value| *value < -1.0e-12);
    let has_positive = crosses.iter().any(|value| *value > 1.0e-12);
    !(has_negative && has_positive)
}

fn render_circle_mesh(
    circle: ExactCircleProfile,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    const SEGMENTS: usize = 32;
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    if [center_x, center_y, radius, height]
        .into_iter()
        .any(|value| !value.is_finite())
        || radius <= 0.0
        || height <= 0.0
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut vertices = Vec::with_capacity(2 + SEGMENTS * 2);
    vertices.push(ExactVertex {
        position_mm: [center_x, center_y, 0.0],
    });
    vertices.push(ExactVertex {
        position_mm: [center_x, center_y, height],
    });
    for index in 0..SEGMENTS {
        let angle = std::f64::consts::TAU * index as f64 / SEGMENTS as f64;
        let point = [
            center_x + radius * angle.cos(),
            center_y + radius * angle.sin(),
        ];
        vertices.push(ExactVertex {
            position_mm: [point[0], point[1], 0.0],
        });
        vertices.push(ExactVertex {
            position_mm: [point[0], point[1], height],
        });
    }
    let mut triangles = Vec::with_capacity(SEGMENTS * 4);
    for index in 0..SEGMENTS {
        let next = (index + 1) % SEGMENTS;
        let bottom = 2 + (index * 2) as u32;
        let top = bottom + 1;
        let next_bottom = 2 + (next * 2) as u32;
        let next_top = next_bottom + 1;
        triangles.extend([
            ExactTriangle {
                vertex_indices: [0, next_bottom, bottom],
                face_role: Some(ExactFaceRole::Bottom),
            },
            ExactTriangle {
                vertex_indices: [1, top, next_top],
                face_role: Some(ExactFaceRole::Top),
            },
            ExactTriangle {
                vertex_indices: [bottom, next_bottom, next_top],
                face_role: Some(ExactFaceRole::CircleSide),
            },
            ExactTriangle {
                vertex_indices: [bottom, next_top, top],
                face_role: Some(ExactFaceRole::CircleSide),
            },
        ]);
    }
    Ok((vertices, triangles))
}

fn render_side_overlapping_circular_cut_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    circle
        .side_overlap(width, depth)
        .or_else(|| circle.outside_side_overlap(width, depth))
        .or_else(|| circle.center_on_side_overlap(width, depth))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let bounds = [
        center_x - radius,
        center_y - radius,
        center_x + radius,
        center_y + radius,
    ];
    let side = if bounds[0] < -1.0e-6 {
        0
    } else if bounds[2] > width + 1.0e-6 {
        1
    } else if bounds[1] < -1.0e-6 {
        2
    } else if bounds[3] > depth + 1.0e-6 {
        3
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let (canonical_width, canonical_depth, canonical_center) = match side {
        0 => (width, depth, [width - center_x, center_y]),
        1 => (width, depth, [center_x, center_y]),
        2 => (depth, width, [depth - center_y, center_x]),
        3 => (depth, width, [center_y, center_x]),
        _ => unreachable!(),
    };
    let distance = canonical_width - canonical_center[0];
    let chord_half = (radius * radius - distance * distance).sqrt();
    let theta = (distance / radius).acos();
    let sweep = -(std::f64::consts::TAU - 2.0 * theta);
    let steps = (sweep.abs() / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
    let mut boundary = vec![
        [0.0, 0.0],
        [canonical_width, 0.0],
        [canonical_width, canonical_center[1] - chord_half],
    ];
    for step in 1..=steps {
        let angle = -theta + sweep * step as f64 / steps as f64;
        boundary.push([
            canonical_center[0] + radius * angle.cos(),
            canonical_center[1] + radius * angle.sin(),
        ]);
    }
    boundary.extend([[canonical_width, canonical_depth], [0.0, canonical_depth]]);
    let to_world = |point: [f64; 2]| match side {
        0 => [width - point[0], point[1]],
        1 => point,
        2 => [point[1], depth - point[0]],
        3 => [point[1], point[0]],
        _ => unreachable!(),
    };
    let mut boundary = boundary.into_iter().map(to_world).collect::<Vec<_>>();
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let arc_center = to_world(canonical_center);
    let (vertices, mut triangles) =
        render_profile_boundary_prism(&boundary, height, Some((arc_center, radius)))?;
    let host_role = if side == 1 {
        ExactFaceRole::West
    } else {
        ExactFaceRole::East
    };
    let host_x = if side == 1 { 0.0 } else { width };
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::ArcSide) {
            triangle.face_role = Some(ExactFaceRole::CutCircle);
        } else if triangle.face_role.is_none()
            && triangle
                .vertex_indices
                .iter()
                .all(|index| (vertices[*index as usize].position_mm[0] - host_x).abs() <= 1.0e-9)
        {
            triangle.face_role = Some(host_role);
        }
    }
    Ok((vertices, triangles))
}

fn render_corner_overlapping_circular_cut_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    circle
        .corner_overlap(width, depth)
        .or_else(|| circle.center_on_corner_overlap(width, depth))
        .or_else(|| circle.outside_corner_overlap(width, depth))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let bounds = [
        center_x - radius,
        center_y - radius,
        center_x + radius,
        center_y + radius,
    ];
    let west = bounds[0] < -1.0e-6;
    let east = bounds[2] > width + 1.0e-6;
    let south = bounds[1] < -1.0e-6;
    let canonical_center = [
        if west { width - center_x } else { center_x },
        if south { depth - center_y } else { center_y },
    ];
    let distance_x = width - canonical_center[0];
    let distance_y = depth - canonical_center[1];
    let right_chord_half = (radius * radius - distance_x * distance_x).sqrt();
    let top_chord_half = (radius * radius - distance_y * distance_y).sqrt();
    let start_angle = (-right_chord_half).atan2(distance_x);
    let end_angle = distance_y.atan2(-top_chord_half);
    let sweep = if distance_x < 0.0 && distance_y < 0.0 {
        end_angle - start_angle
    } else {
        end_angle - start_angle - std::f64::consts::TAU
    };
    if sweep >= 0.0 || sweep <= -std::f64::consts::PI {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let steps = (sweep.abs() / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
    let mut boundary = vec![
        [0.0, 0.0],
        [width, 0.0],
        [width, canonical_center[1] - right_chord_half],
    ];
    for step in 1..=steps {
        let angle = start_angle + sweep * step as f64 / steps as f64;
        boundary.push([
            canonical_center[0] + radius * angle.cos(),
            canonical_center[1] + radius * angle.sin(),
        ]);
    }
    boundary.push([0.0, depth]);
    let to_world = |point: [f64; 2]| {
        [
            if west { width - point[0] } else { point[0] },
            if south { depth - point[1] } else { point[1] },
        ]
    };
    let mut boundary = boundary.into_iter().map(to_world).collect::<Vec<_>>();
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let arc_center = to_world(canonical_center);
    let (vertices, mut triangles) =
        render_profile_boundary_prism(&boundary, height, Some((arc_center, radius)))?;
    let host_role = if east {
        ExactFaceRole::West
    } else {
        ExactFaceRole::East
    };
    let host_x = if east { 0.0 } else { width };
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::ArcSide) {
            triangle.face_role = Some(ExactFaceRole::CutCircle);
        } else if triangle.face_role.is_none()
            && triangle
                .vertex_indices
                .iter()
                .all(|index| (vertices[*index as usize].position_mm[0] - host_x).abs() <= 1.0e-9)
        {
            triangle.face_role = Some(host_role);
        }
    }
    Ok((vertices, triangles))
}

fn render_side_overlapping_circular_union_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    circle
        .side_overlap(width, depth)
        .or_else(|| circle.outside_side_overlap(width, depth))
        .or_else(|| circle.center_on_side_overlap(width, depth))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let bounds = [
        center_x - radius,
        center_y - radius,
        center_x + radius,
        center_y + radius,
    ];
    let side = if bounds[0] < -1.0e-6 {
        0
    } else if bounds[2] > width + 1.0e-6 {
        1
    } else if bounds[1] < -1.0e-6 {
        2
    } else if bounds[3] > depth + 1.0e-6 {
        3
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let (canonical_width, canonical_depth, canonical_center) = match side {
        0 => (width, depth, [width - center_x, center_y]),
        1 => (width, depth, [center_x, center_y]),
        2 => (depth, width, [depth - center_y, center_x]),
        3 => (depth, width, [center_y, center_x]),
        _ => unreachable!(),
    };
    let distance = canonical_width - canonical_center[0];
    let chord_half = (radius * radius - distance * distance).sqrt();
    let theta = (distance / radius).acos();
    let steps = (2.0 * theta / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
    let mut boundary = vec![
        [0.0, 0.0],
        [canonical_width, 0.0],
        [canonical_width, canonical_center[1] - chord_half],
    ];
    for step in 1..=steps {
        let angle = -theta + 2.0 * theta * step as f64 / steps as f64;
        boundary.push([
            canonical_center[0] + radius * angle.cos(),
            canonical_center[1] + radius * angle.sin(),
        ]);
    }
    boundary.extend([[canonical_width, canonical_depth], [0.0, canonical_depth]]);
    let to_world = |point: [f64; 2]| match side {
        0 => [width - point[0], point[1]],
        1 => point,
        2 => [point[1], depth - point[0]],
        3 => [point[1], point[0]],
        _ => unreachable!(),
    };
    let mut boundary = boundary.into_iter().map(to_world).collect::<Vec<_>>();
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let arc_center = to_world(canonical_center);
    let (vertices, mut triangles) =
        render_profile_boundary_prism(&boundary, height, Some((arc_center, radius)))?;
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::ArcSide) {
            triangle.face_role = Some(ExactFaceRole::CircleSide);
        }
    }
    Ok((vertices, triangles))
}

fn render_corner_overlapping_circular_union_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    circle
        .corner_overlap(width, depth)
        .or_else(|| circle.center_on_corner_overlap(width, depth))
        .or_else(|| circle.outside_corner_overlap(width, depth))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let bounds = [
        center_x - radius,
        center_y - radius,
        center_x + radius,
        center_y + radius,
    ];
    let west = bounds[0] < -1.0e-6;
    let south = bounds[1] < -1.0e-6;
    let canonical_center = [
        if west { width - center_x } else { center_x },
        if south { depth - center_y } else { center_y },
    ];
    let distance_x = width - canonical_center[0];
    let distance_y = depth - canonical_center[1];
    let right_chord_half = (radius * radius - distance_x * distance_x).sqrt();
    let top_chord_half = (radius * radius - distance_y * distance_y).sqrt();
    let right_angle = (-right_chord_half).atan2(distance_x);
    let mut top_angle = distance_y.atan2(-top_chord_half);
    if top_angle <= right_angle {
        top_angle += std::f64::consts::TAU;
    }
    let sweep = top_angle - right_angle;
    if sweep <= std::f64::consts::PI || sweep >= std::f64::consts::TAU {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let steps = (sweep / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
    let mut boundary = vec![
        [0.0, 0.0],
        [width, 0.0],
        [width, canonical_center[1] - right_chord_half],
    ];
    for step in 1..=steps {
        let angle = right_angle + sweep * step as f64 / steps as f64;
        boundary.push([
            canonical_center[0] + radius * angle.cos(),
            canonical_center[1] + radius * angle.sin(),
        ]);
    }
    boundary.push([0.0, depth]);
    let to_world = |point: [f64; 2]| {
        [
            if west { width - point[0] } else { point[0] },
            if south { depth - point[1] } else { point[1] },
        ]
    };
    let mut boundary = boundary.into_iter().map(to_world).collect::<Vec<_>>();
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let arc_center = to_world(canonical_center);
    let (vertices, mut triangles) =
        render_profile_boundary_prism(&boundary, height, Some((arc_center, radius)))?;
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::ArcSide) {
            triangle.face_role = Some(ExactFaceRole::CircleSide);
        }
    }
    Ok((vertices, triangles))
}

fn render_side_overlapping_circular_intersection_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    circle
        .side_overlap(width, depth)
        .or_else(|| circle.outside_side_overlap(width, depth))
        .or_else(|| circle.center_on_side_overlap(width, depth))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let bounds = [
        center_x - radius,
        center_y - radius,
        center_x + radius,
        center_y + radius,
    ];
    let side = if bounds[0] < -1.0e-6 {
        0
    } else if bounds[2] > width + 1.0e-6 {
        1
    } else if bounds[1] < -1.0e-6 {
        2
    } else if bounds[3] > depth + 1.0e-6 {
        3
    } else {
        return Err(ExactProductError::InvalidWorkerEvidence);
    };
    let (canonical_width, canonical_center) = match side {
        0 => (width, [width - center_x, center_y]),
        1 => (width, [center_x, center_y]),
        2 => (depth, [depth - center_y, center_x]),
        3 => (depth, [center_y, center_x]),
        _ => unreachable!(),
    };
    let distance = canonical_width - canonical_center[0];
    let chord_half = (radius * radius - distance * distance).sqrt();
    let theta = (distance / radius).acos();
    let sweep = -(std::f64::consts::TAU - 2.0 * theta);
    let steps = (sweep.abs() / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
    let mut boundary = vec![[canonical_width, canonical_center[1] - chord_half]];
    for step in 1..=steps {
        let angle = -theta + sweep * step as f64 / steps as f64;
        boundary.push([
            canonical_center[0] + radius * angle.cos(),
            canonical_center[1] + radius * angle.sin(),
        ]);
    }
    let to_world = |point: [f64; 2]| match side {
        0 => [width - point[0], point[1]],
        1 => point,
        2 => [point[1], depth - point[0]],
        3 => [point[1], point[0]],
        _ => unreachable!(),
    };
    let mut boundary = boundary.into_iter().map(to_world).collect::<Vec<_>>();
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let arc_center = to_world(canonical_center);
    let (vertices, mut triangles) =
        render_profile_boundary_prism(&boundary, height, Some((arc_center, radius)))?;
    let on_chord = |position: [f64; 3]| match side {
        0 => position[0].abs() <= 1.0e-9,
        1 => (position[0] - width).abs() <= 1.0e-9,
        2 => position[1].abs() <= 1.0e-9,
        3 => (position[1] - depth).abs() <= 1.0e-9,
        _ => unreachable!(),
    };
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::ArcSide) {
            triangle.face_role = if triangle
                .vertex_indices
                .iter()
                .all(|index| on_chord(vertices[*index as usize].position_mm))
            {
                None
            } else {
                Some(ExactFaceRole::CircleSide)
            };
        }
    }
    Ok((vertices, triangles))
}

fn render_corner_overlapping_circular_intersection_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    circle
        .corner_overlap(width, depth)
        .or_else(|| circle.center_on_corner_overlap(width, depth))
        .or_else(|| circle.outside_corner_overlap(width, depth))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let bounds = [
        center_x - radius,
        center_y - radius,
        center_x + radius,
        center_y + radius,
    ];
    let west = bounds[0] < -1.0e-6;
    let south = bounds[1] < -1.0e-6;
    let canonical_center = [
        if west { width - center_x } else { center_x },
        if south { depth - center_y } else { center_y },
    ];
    let distance_x = width - canonical_center[0];
    let distance_y = depth - canonical_center[1];
    let right_chord_half = (radius * radius - distance_x * distance_x).sqrt();
    let top_chord_half = (radius * radius - distance_y * distance_y).sqrt();
    let right_angle = (-right_chord_half).atan2(distance_x);
    let top_angle = distance_y.atan2(-top_chord_half);
    let sweep = if distance_x < 0.0 && distance_y < 0.0 {
        right_angle - top_angle
    } else {
        right_angle + std::f64::consts::TAU - top_angle
    };
    if sweep <= 0.0 || sweep >= std::f64::consts::PI {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let steps = (sweep / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
    let mut boundary = vec![
        [width, canonical_center[1] - right_chord_half],
        [width, depth],
        [canonical_center[0] - top_chord_half, depth],
    ];
    for step in 1..steps {
        let angle = top_angle + sweep * step as f64 / steps as f64;
        boundary.push([
            canonical_center[0] + radius * angle.cos(),
            canonical_center[1] + radius * angle.sin(),
        ]);
    }
    let to_world = |point: [f64; 2]| {
        [
            if west { width - point[0] } else { point[0] },
            if south { depth - point[1] } else { point[1] },
        ]
    };
    let mut boundary = boundary.into_iter().map(to_world).collect::<Vec<_>>();
    if polygon_signed_area(&boundary) < 0.0 {
        boundary.reverse();
    }
    let arc_center = to_world(canonical_center);
    let (vertices, mut triangles) =
        render_profile_boundary_prism(&boundary, height, Some((arc_center, radius)))?;
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::ArcSide) {
            triangle.face_role = Some(ExactFaceRole::CircleSide);
        }
    }
    Ok((vertices, triangles))
}

fn render_side_overlapping_circular_split_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let (mut vertices, mut triangles) =
        render_side_overlapping_circular_cut_mesh(width, depth, height, circle)?;
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::CutCircle) {
            triangle.face_role = Some(ExactFaceRole::CircleSide);
        }
    }
    let (inner_vertices, mut inner_triangles) =
        render_side_overlapping_circular_intersection_mesh(width, depth, height, circle)?;
    let offset = vertices.len() as u32;
    for triangle in &mut inner_triangles {
        triangle.vertex_indices = triangle.vertex_indices.map(|index| index + offset);
    }
    vertices.extend(inner_vertices);
    triangles.extend(inner_triangles);
    Ok((vertices, triangles))
}

fn render_corner_overlapping_circular_split_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let (mut vertices, mut triangles) =
        render_corner_overlapping_circular_cut_mesh(width, depth, height, circle)?;
    for triangle in &mut triangles {
        if triangle.face_role == Some(ExactFaceRole::CutCircle) {
            triangle.face_role = Some(ExactFaceRole::CircleSide);
        }
    }
    let (inner_vertices, mut inner_triangles) =
        render_corner_overlapping_circular_intersection_mesh(width, depth, height, circle)?;
    let offset = vertices.len() as u32;
    for triangle in &mut inner_triangles {
        triangle.vertex_indices = triangle.vertex_indices.map(|index| index + offset);
    }
    vertices.extend(inner_vertices);
    triangles.extend(inner_triangles);
    Ok((vertices, triangles))
}

fn render_overlapping_circular_pocket_mesh(
    width: f64,
    depth: f64,
    height: f64,
    pocket_depth: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if !pocket_depth.is_finite() || pocket_depth <= 0.0 || pocket_depth >= height {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    let side_overlap = circle
        .side_overlap(width, depth)
        .or_else(|| circle.outside_side_overlap(width, depth))
        .or_else(|| circle.center_on_side_overlap(width, depth));
    let corner_overlap = circle
        .corner_overlap(width, depth)
        .or_else(|| circle.center_on_corner_overlap(width, depth))
        .or_else(|| circle.outside_corner_overlap(width, depth));
    if side_overlap.is_none() && corner_overlap.is_none() {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let bounds = [
        center_x - radius,
        center_y - radius,
        center_x + radius,
        center_y + radius,
    ];
    let east_overlap = bounds[2] > width + 1.0e-6;
    let mut floor_boundary = if side_overlap.is_some() {
        let side = if bounds[0] < -1.0e-6 {
            0
        } else if east_overlap {
            1
        } else if bounds[1] < -1.0e-6 {
            2
        } else if bounds[3] > depth + 1.0e-6 {
            3
        } else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        let (canonical_width, canonical_center) = match side {
            0 => (width, [width - center_x, center_y]),
            1 => (width, [center_x, center_y]),
            2 => (depth, [depth - center_y, center_x]),
            3 => (depth, [center_y, center_x]),
            _ => unreachable!(),
        };
        let distance = canonical_width - canonical_center[0];
        let chord_half = (radius * radius - distance * distance).sqrt();
        let theta = (distance / radius).acos();
        let sweep = std::f64::consts::TAU - 2.0 * theta;
        let steps = (sweep / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
        let mut boundary = vec![
            [canonical_width, canonical_center[1] - chord_half],
            [canonical_width, canonical_center[1] + chord_half],
        ];
        for step in 1..steps {
            let angle = theta + sweep * step as f64 / steps as f64;
            boundary.push([
                canonical_center[0] + radius * angle.cos(),
                canonical_center[1] + radius * angle.sin(),
            ]);
        }
        let to_world = |point: [f64; 2]| match side {
            0 => [width - point[0], point[1]],
            1 => point,
            2 => [point[1], depth - point[0]],
            3 => [point[1], point[0]],
            _ => unreachable!(),
        };
        boundary.into_iter().map(to_world).collect::<Vec<_>>()
    } else {
        let west = bounds[0] < -1.0e-6;
        let south = bounds[1] < -1.0e-6;
        let canonical_center = [
            if west { width - center_x } else { center_x },
            if south { depth - center_y } else { center_y },
        ];
        let distance_x = width - canonical_center[0];
        let distance_y = depth - canonical_center[1];
        let right_chord_half = (radius * radius - distance_x * distance_x).sqrt();
        let top_chord_half = (radius * radius - distance_y * distance_y).sqrt();
        let start_angle = (-right_chord_half).atan2(distance_x);
        let end_angle = distance_y.atan2(-top_chord_half);
        let sweep = if distance_x < 0.0 && distance_y < 0.0 {
            start_angle - end_angle
        } else {
            std::f64::consts::TAU - (end_angle - start_angle)
        };
        if sweep <= 0.0 || sweep >= std::f64::consts::PI {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        let steps = (sweep / std::f64::consts::TAU * 64.0).ceil().max(1.0) as usize;
        let mut boundary = vec![
            [width, canonical_center[1] - right_chord_half],
            [width, depth],
            [canonical_center[0] - top_chord_half, depth],
        ];
        for step in 1..steps {
            let angle = end_angle + sweep * step as f64 / steps as f64;
            boundary.push([
                canonical_center[0] + radius * angle.cos(),
                canonical_center[1] + radius * angle.sin(),
            ]);
        }
        let to_world = |point: [f64; 2]| {
            [
                if west { width - point[0] } else { point[0] },
                if south { depth - point[1] } else { point[1] },
            ]
        };
        boundary.into_iter().map(to_world).collect::<Vec<_>>()
    };
    if polygon_signed_area(&floor_boundary) < 0.0 {
        floor_boundary.reverse();
    }

    let floor_z = height - pocket_depth;
    let (mut vertices, mut triangles) = render_circular_cut_mesh(width, depth, height, circle)?;
    for vertex in &mut vertices {
        if vertex.position_mm[2] == 0.0 {
            vertex.position_mm[2] = floor_z;
        }
    }
    triangles.retain(|triangle| triangle.face_role != Some(ExactFaceRole::Bottom));

    let tolerance = 1.0e-6;
    let perimeter_position = |point: [f64; 2]| {
        if point[1].abs() <= tolerance {
            Some(point[0])
        } else if (point[0] - width).abs() <= tolerance {
            Some(width + point[1])
        } else if (point[1] - depth).abs() <= tolerance {
            Some(2.0 * width + depth - point[0])
        } else if point[0].abs() <= tolerance {
            Some(2.0 * (width + depth) - point[1])
        } else {
            None
        }
    };
    let mut positioned_boundary = vec![
        (0.0, [0.0, 0.0]),
        (width, [width, 0.0]),
        (width + depth, [width, depth]),
        (2.0 * width + depth, [0.0, depth]),
    ];
    for point in floor_boundary.iter().copied() {
        if let Some(position) = perimeter_position(point)
            && !positioned_boundary
                .iter()
                .any(|(existing, _)| (existing - position).abs() <= tolerance)
        {
            positioned_boundary.push((position, point));
        }
    }
    positioned_boundary.sort_by(|left, right| left.0.total_cmp(&right.0));
    let lower_boundary = positioned_boundary
        .into_iter()
        .map(|(_, point)| point)
        .collect::<Vec<_>>();
    let (lower_vertices, mut lower_triangles) =
        render_profile_boundary_prism(&lower_boundary, floor_z, None)?;
    lower_triangles.retain(|triangle| triangle.face_role != Some(ExactFaceRole::Top));
    let (host_role, host_x) = if east_overlap {
        (ExactFaceRole::West, 0.0)
    } else {
        (ExactFaceRole::East, width)
    };
    for triangle in &mut lower_triangles {
        if triangle
            .vertex_indices
            .iter()
            .all(|index| (lower_vertices[*index as usize].position_mm[0] - host_x).abs() <= 1.0e-9)
        {
            triangle.face_role = Some(host_role);
        }
        triangle.vertex_indices = triangle
            .vertex_indices
            .map(|index| index + vertices.len() as u32);
    }
    vertices.extend(lower_vertices);
    triangles.extend(lower_triangles);

    let floor_start = vertices.len() as u32;
    vertices.extend(floor_boundary.iter().map(|point| ExactVertex {
        position_mm: [point[0], point[1], floor_z],
    }));
    for [a, b, c] in triangulate_polygon(&floor_boundary)? {
        triangles.push(ExactTriangle {
            vertex_indices: [floor_start + a, floor_start + b, floor_start + c],
            face_role: Some(ExactFaceRole::PocketFloor),
        });
    }

    let weld_tolerance = width.min(depth) * 1.0e-9;
    let mut welded_vertices: Vec<ExactVertex> = Vec::new();
    let remap = vertices
        .into_iter()
        .map(|vertex| {
            welded_vertices
                .iter()
                .position(|candidate| {
                    candidate
                        .position_mm
                        .into_iter()
                        .zip(vertex.position_mm)
                        .all(|(left, right)| (left - right).abs() <= weld_tolerance)
                })
                .map_or_else(
                    || {
                        welded_vertices.push(vertex);
                        (welded_vertices.len() - 1) as u32
                    },
                    |index| index as u32,
                )
        })
        .collect::<Vec<_>>();
    let welded_triangles = triangles
        .into_iter()
        .filter_map(|mut triangle| {
            triangle.vertex_indices = triangle.vertex_indices.map(|index| remap[index as usize]);
            let [a, b, c] = triangle.vertex_indices;
            (a != b && b != c && c != a).then_some(triangle)
        })
        .collect();
    Ok((welded_vertices, welded_triangles))
}

fn render_circular_cut_mesh(
    width: f64,
    depth: f64,
    height: f64,
    circle: ExactCircleProfile,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    const SEGMENTS: usize = 32;
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits);
    if [width, depth, height, center_x, center_y, radius]
        .into_iter()
        .any(|value| !value.is_finite())
        || width <= 0.0
        || depth <= 0.0
        || height <= 0.0
        || radius <= 0.0
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    if circle.side_overlap(width, depth).is_some()
        || circle.outside_side_overlap(width, depth).is_some()
        || circle.center_on_side_overlap(width, depth).is_some()
    {
        return render_side_overlapping_circular_cut_mesh(width, depth, height, circle);
    }
    if circle.corner_overlap(width, depth).is_some()
        || circle.center_on_corner_overlap(width, depth).is_some()
        || circle.outside_corner_overlap(width, depth).is_some()
    {
        return render_corner_overlapping_circular_cut_mesh(width, depth, height, circle);
    }
    if center_x - radius <= 0.0
        || center_y - radius <= 0.0
        || center_x + radius >= width
        || center_y + radius >= depth
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut vertices = Vec::with_capacity(SEGMENTS * 4);
    for index in 0..SEGMENTS {
        let angle = std::f64::consts::TAU * index as f64 / SEGMENTS as f64;
        let direction = [angle.cos(), angle.sin()];
        let tx = if direction[0] > 0.0 {
            (width - center_x) / direction[0]
        } else if direction[0] < 0.0 {
            -center_x / direction[0]
        } else {
            f64::INFINITY
        };
        let ty = if direction[1] > 0.0 {
            (depth - center_y) / direction[1]
        } else if direction[1] < 0.0 {
            -center_y / direction[1]
        } else {
            f64::INFINITY
        };
        let outer_distance = tx.min(ty);
        let outer = [
            center_x + outer_distance * direction[0],
            center_y + outer_distance * direction[1],
        ];
        let inner = [
            center_x + radius * direction[0],
            center_y + radius * direction[1],
        ];
        for position_mm in [
            [outer[0], outer[1], 0.0],
            [inner[0], inner[1], 0.0],
            [outer[0], outer[1], height],
            [inner[0], inner[1], height],
        ] {
            vertices.push(ExactVertex { position_mm });
        }
    }
    let mut triangles = Vec::with_capacity(SEGMENTS * 8);
    for index in 0..SEGMENTS {
        let next = (index + 1) % SEGMENTS;
        let outer_bottom = (index * 4) as u32;
        let inner_bottom = outer_bottom + 1;
        let outer_top = outer_bottom + 2;
        let inner_top = outer_bottom + 3;
        let next_outer_bottom = (next * 4) as u32;
        let next_inner_bottom = next_outer_bottom + 1;
        let next_outer_top = next_outer_bottom + 2;
        let next_inner_top = next_outer_bottom + 3;
        let east = (vertices[outer_bottom as usize].position_mm[0] - width).abs() <= 1.0e-9
            && (vertices[next_outer_bottom as usize].position_mm[0] - width).abs() <= 1.0e-9;
        let outer_role = east.then_some(ExactFaceRole::East);
        triangles.extend([
            ExactTriangle {
                vertex_indices: [outer_bottom, inner_bottom, next_inner_bottom],
                face_role: Some(ExactFaceRole::Bottom),
            },
            ExactTriangle {
                vertex_indices: [outer_bottom, next_inner_bottom, next_outer_bottom],
                face_role: Some(ExactFaceRole::Bottom),
            },
            ExactTriangle {
                vertex_indices: [outer_top, next_outer_top, next_inner_top],
                face_role: Some(ExactFaceRole::Top),
            },
            ExactTriangle {
                vertex_indices: [outer_top, next_inner_top, inner_top],
                face_role: Some(ExactFaceRole::Top),
            },
            ExactTriangle {
                vertex_indices: [outer_bottom, next_outer_bottom, next_outer_top],
                face_role: outer_role,
            },
            ExactTriangle {
                vertex_indices: [outer_bottom, next_outer_top, outer_top],
                face_role: outer_role,
            },
            ExactTriangle {
                vertex_indices: [inner_bottom, inner_top, next_inner_top],
                face_role: Some(ExactFaceRole::CutCircle),
            },
            ExactTriangle {
                vertex_indices: [inner_bottom, next_inner_top, next_inner_bottom],
                face_role: Some(ExactFaceRole::CutCircle),
            },
        ]);
    }
    Ok((vertices, triangles))
}

fn is_simple_linear_profile(segments: &[ProfileSegment]) -> bool {
    let points = segments
        .iter()
        .map(ProfileSegment::start_mm)
        .collect::<Vec<_>>();
    if points.iter().enumerate().any(|(index, point)| {
        points[index + 1..].iter().any(|candidate| {
            (point[0] - candidate[0]).abs() <= 1.0e-9 && (point[1] - candidate[1]).abs() <= 1.0e-9
        })
    }) {
        return false;
    }
    for left in 0..segments.len() {
        let left_next = (left + 1) % segments.len();
        for right in (left + 1)..segments.len() {
            let right_next = (right + 1) % segments.len();
            if left == right_next || left_next == right {
                continue;
            }
            if planar_line_segments_intersect(
                segments[left].start_mm(),
                segments[left].end_mm(),
                segments[right].start_mm(),
                segments[right].end_mm(),
            ) {
                return false;
            }
        }
    }
    true
}

fn planar_line_segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let cross = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        (end[0] - start[0]) * (point[1] - start[1]) - (end[1] - start[1]) * (point[0] - start[0])
    };
    let on_segment = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        point[0] >= start[0].min(end[0]) - 1.0e-9
            && point[0] <= start[0].max(end[0]) + 1.0e-9
            && point[1] >= start[1].min(end[1]) - 1.0e-9
            && point[1] <= start[1].max(end[1]) + 1.0e-9
    };
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    if ((ab_c > 1.0e-9 && ab_d < -1.0e-9) || (ab_c < -1.0e-9 && ab_d > 1.0e-9))
        && ((cd_a > 1.0e-9 && cd_b < -1.0e-9) || (cd_a < -1.0e-9 && cd_b > 1.0e-9))
    {
        return true;
    }
    (ab_c.abs() <= 1.0e-9 && on_segment(a, b, c))
        || (ab_d.abs() <= 1.0e-9 && on_segment(a, b, d))
        || (cd_a.abs() <= 1.0e-9 && on_segment(c, d, a))
        || (cd_b.abs() <= 1.0e-9 && on_segment(c, d, b))
}

#[must_use]
pub fn strict_convex_line_arc_profile_bounds(
    segments: &[ProfileSegment],
    closed: bool,
) -> Option<[f64; 4]> {
    let profile = exact_mixed_profile(segments, closed)?;
    profile
        .is_strict_convex_line_arc_profile()
        .then(|| profile.bounds_bits.map(f64::from_bits))
}

#[must_use]
pub fn line_arc_circle_side_overlap(
    segments: &[ProfileSegment],
    closed: bool,
    width: f64,
    depth: f64,
) -> Option<(f64, [f64; 4])> {
    exact_circle_profile(segments, closed)?.side_overlap(width, depth)
}

#[must_use]
pub fn line_arc_circle_corner_overlap(
    segments: &[ProfileSegment],
    closed: bool,
    width: f64,
    depth: f64,
) -> Option<(f64, [f64; 4])> {
    exact_circle_profile(segments, closed)?.corner_overlap(width, depth)
}

#[must_use]
pub fn line_arc_d_arc_only_side_overlap(
    segments: &[ProfileSegment],
    closed: bool,
    width: f64,
    depth: f64,
) -> Option<(f64, [f64; 4])> {
    exact_mixed_profile(segments, closed)?.d_profile_arc_only_clipped_side_overlap(width, depth)
}

#[must_use]
pub fn line_arc_capsule_profile_bounds(
    segments: &[ProfileSegment],
    closed: bool,
) -> Option<[f64; 4]> {
    let profile = exact_mixed_profile(segments, closed)?;
    profile
        .is_line_arc_capsule_profile()
        .then(|| profile.bounds_bits.map(f64::from_bits))
}

#[must_use]
pub fn line_arc_capsule_side_overlap(
    segments: &[ProfileSegment],
    closed: bool,
    width: f64,
    depth: f64,
) -> Option<(f64, [f64; 4])> {
    exact_mixed_profile(segments, closed)?.capsule_side_overlap(width, depth)
}

#[must_use]
pub fn line_arc_capsule_corner_overlap(
    segments: &[ProfileSegment],
    closed: bool,
    width: f64,
    depth: f64,
) -> Option<(f64, [f64; 4])> {
    exact_mixed_profile(segments, closed)?.capsule_corner_overlap(width, depth)
}

#[must_use]
pub fn line_arc_profile_bounds(segments: &[ProfileSegment], closed: bool) -> Option<[f64; 4]> {
    exact_mixed_profile(segments, closed).map(|profile| profile.bounds_bits.map(f64::from_bits))
}

#[must_use]
pub fn is_line_arc_capsule_profile(segments: &[ProfileSegment], closed: bool) -> bool {
    line_arc_capsule_profile_bounds(segments, closed).is_some()
}

fn exact_mixed_profile(segments: &[ProfileSegment], closed: bool) -> Option<ExactMixedProfile> {
    let line_only = segments
        .iter()
        .all(|segment| matches!(segment, ProfileSegment::Line { .. }));
    if !closed
        || !(2..=64).contains(&segments.len())
        || !segments
            .iter()
            .any(|segment| matches!(segment, ProfileSegment::Line { .. }))
        || (line_only && (segments.len() < 3 || !is_simple_linear_profile(segments)))
        || segments
            .windows(2)
            .any(|pair| pair[0].end_mm() != pair[1].start_mm())
        || segments.last()?.end_mm() != segments.first()?.start_mm()
    {
        return None;
    }
    let mut exact_segments = Vec::with_capacity(segments.len());
    let mut points = Vec::new();
    let mut signed_area = 0.0;
    for segment in segments {
        match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                signed_area += 0.5 * (start_mm[0] * end_mm[1] - end_mm[0] * start_mm[1]);
                points.extend([*start_mm, *end_mm]);
                exact_segments.push(ExactProfileSegment::Line {
                    start_bits: start_mm.map(f64::to_bits),
                    end_bits: end_mm.map(f64::to_bits),
                });
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
                let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
                let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)?;
                let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                let end_radius = (end_mm[0] - center_mm[0]).hypot(end_mm[1] - center_mm[1]);
                if !radius.is_finite()
                    || radius < 0.01
                    || (radius - end_radius).abs() > 1.0e-9 * radius.max(end_radius).max(1.0)
                {
                    return None;
                }
                signed_area += 0.5
                    * (radius * radius * sweep + center_mm[0] * (end_mm[1] - start_mm[1])
                        - center_mm[1] * (end_mm[0] - start_mm[0]));
                points.extend([*start_mm, *end_mm]);
                for quadrant in [
                    0.0,
                    std::f64::consts::FRAC_PI_2,
                    std::f64::consts::PI,
                    3.0 * std::f64::consts::FRAC_PI_2,
                ] {
                    if angle_on_directed_arc(start_angle, sweep, quadrant) {
                        points.push([
                            center_mm[0] + radius * quadrant.cos(),
                            center_mm[1] + radius * quadrant.sin(),
                        ]);
                    }
                }
                exact_segments.push(ExactProfileSegment::CircularArc {
                    start_bits: start_mm.map(f64::to_bits),
                    end_bits: end_mm.map(f64::to_bits),
                    center_bits: center_mm.map(f64::to_bits),
                    clockwise: *clockwise,
                });
            }
        }
    }
    let area = signed_area.abs();
    if !area.is_finite() || area <= 1.0e-9 {
        return None;
    }
    let bounds = points.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |mut bounds, point| {
            bounds[0] = bounds[0].min(point[0]);
            bounds[1] = bounds[1].min(point[1]);
            bounds[2] = bounds[2].max(point[0]);
            bounds[3] = bounds[3].max(point[1]);
            bounds
        },
    );
    if bounds.into_iter().any(|value| !value.is_finite())
        || bounds[2] <= bounds[0]
        || bounds[3] <= bounds[1]
    {
        return None;
    }
    Some(ExactMixedProfile {
        segments: exact_segments,
        bounds_bits: bounds.map(f64::to_bits),
        area_bits: area.to_bits(),
    })
}

fn directed_arc_sweep(start: f64, end: f64, clockwise: bool) -> Option<f64> {
    if !start.is_finite() || !end.is_finite() {
        return None;
    }
    let mut sweep = end - start;
    if clockwise {
        while sweep >= 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else {
        while sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
    }
    (sweep.abs() > 1.0e-12 && sweep.abs() < std::f64::consts::TAU - 1.0e-12).then_some(sweep)
}

fn angle_on_directed_arc(start: f64, sweep: f64, candidate: f64) -> bool {
    if sweep > 0.0 {
        (candidate - start).rem_euclid(std::f64::consts::TAU) <= sweep + 1.0e-12
    } else {
        (start - candidate).rem_euclid(std::f64::consts::TAU) <= -sweep + 1.0e-12
    }
}

fn exact_circle_profile(segments: &[ProfileSegment], closed: bool) -> Option<ExactCircleProfile> {
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
    if !closed
        || first_start != second_end
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
    if !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    Some(ExactCircleProfile {
        center_x_bits: first_center[0].to_bits(),
        center_y_bits: first_center[1].to_bits(),
        radius_bits: radius.to_bits(),
        clockwise: *first_clockwise,
    })
}

fn rectangular_union_bounds(base_width: f64, base_depth: f64, tool: [f64; 4]) -> Option<[f64; 4]> {
    let [tool_min_x, tool_min_y, tool_max_x, tool_max_y] = tool;
    if [
        base_width, base_depth, tool_min_x, tool_min_y, tool_max_x, tool_max_y,
    ]
    .into_iter()
    .any(|value| !value.is_finite())
        || base_width <= 0.0
        || base_depth <= 0.0
        || tool_min_x >= tool_max_x
        || tool_min_y >= tool_max_y
    {
        return None;
    }
    let overlap_x = base_width.min(tool_max_x) - 0.0_f64.max(tool_min_x);
    let overlap_y = base_depth.min(tool_max_y) - 0.0_f64.max(tool_min_y);
    if overlap_x <= 1.0e-6 || overlap_y <= 1.0e-6 {
        return None;
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
    ((union_area - bounds_area).abs() <= tolerance
        && (bounds[0] < -1.0e-6
            || bounds[1] < -1.0e-6
            || bounds[2] > base_width + 1.0e-6
            || bounds[3] > base_depth + 1.0e-6))
        .then_some(bounds)
}

fn rectangular_intersection_bounds(
    base_width: f64,
    base_depth: f64,
    tool: [f64; 4],
) -> Option<[f64; 4]> {
    let [tool_min_x, tool_min_y, tool_max_x, tool_max_y] = tool;
    if [
        base_width, base_depth, tool_min_x, tool_min_y, tool_max_x, tool_max_y,
    ]
    .into_iter()
    .any(|value| !value.is_finite())
        || base_width <= 0.0
        || base_depth <= 0.0
        || tool_min_x >= tool_max_x
        || tool_min_y >= tool_max_y
    {
        return None;
    }
    let bounds = [
        0.0_f64.max(tool_min_x),
        0.0_f64.max(tool_min_y),
        base_width.min(tool_max_x),
        base_depth.min(tool_max_y),
    ];
    (bounds[2] - bounds[0] > 1.0e-6 && bounds[3] - bounds[1] > 1.0e-6).then_some(bounds)
}

fn rectangular_split_supported(base_width: f64, base_depth: f64, tool: [f64; 4]) -> bool {
    let Some([overlap_min_x, overlap_min_y, overlap_max_x, overlap_max_y]) =
        rectangular_intersection_bounds(base_width, base_depth, tool)
    else {
        return false;
    };
    let [tool_min_x, tool_min_y, tool_max_x, tool_max_y] = tool;
    overlap_max_x - overlap_min_x > 1.0e-6
        && overlap_max_y - overlap_min_y > 1.0e-6
        && ((tool_min_x > 1.0e-6 && tool_min_x < base_width - 1.0e-6)
            || (tool_max_x > 1.0e-6 && tool_max_x < base_width - 1.0e-6)
            || (tool_min_y > 1.0e-6 && tool_min_y < base_depth - 1.0e-6)
            || (tool_max_y > 1.0e-6 && tool_max_y < base_depth - 1.0e-6))
}

fn selected_solved_region(
    sketch: &crate::sketch::SketchSpec,
    region_id: crate::sketch::SketchRegionId,
) -> Result<SolvedSketchRegion, ExactProductError> {
    sketch
        .solved_regions()
        .map_err(|_| ExactProductError::UnsupportedProfile)?
        .into_iter()
        .find(|region| region.id == region_id)
        .ok_or(ExactProductError::UnsupportedProfile)
}

struct ExactRegionProfile {
    width_mm: f64,
    depth_mm: f64,
    circle: Option<ExactCircleProfile>,
    mixed_profile: Option<ExactMixedProfile>,
    frame: [f64; 12],
}

fn exact_region_profile(
    region: SolvedSketchRegion,
    workplane: &WorkplaneSpec,
) -> Result<ExactRegionProfile, ExactProductError> {
    if !region.holes.is_empty() {
        return Err(ExactProductError::UnsupportedProfile);
    }
    let frame = workplane.frame;
    let mut transform = [
        frame.origin_mm[0],
        frame.origin_mm[1],
        frame.origin_mm[2],
        frame.x_axis[0],
        frame.x_axis[1],
        frame.x_axis[2],
        frame.y_axis[0],
        frame.y_axis[1],
        frame.y_axis[2],
        frame.normal[0],
        frame.normal[1],
        frame.normal[2],
    ];
    if let SolvedSketchRegionProfile::Polyline(points) = &region.outer
        && let Some([min_x, min_y, max_x, max_y]) = rectangle_bounds(points)
    {
        for (axis, coordinate) in transform.iter_mut().take(3).enumerate() {
            *coordinate += frame.x_axis[axis] * min_x + frame.y_axis[axis] * min_y;
        }
        return Ok(ExactRegionProfile {
            width_mm: max_x - min_x,
            depth_mm: max_y - min_y,
            circle: None,
            mixed_profile: None,
            frame: transform,
        });
    }
    match &region.outer {
        SolvedSketchRegionProfile::Polyline(_) | SolvedSketchRegionProfile::Boundary(_) => {
            let profile = exact_mixed_profile_from_solved(&region.outer)
                .ok_or(ExactProductError::UnsupportedProfile)?;
            let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
            Ok(ExactRegionProfile {
                width_mm: max_x - min_x,
                depth_mm: max_y - min_y,
                circle: None,
                mixed_profile: Some(profile),
                frame: transform,
            })
        }
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => Ok(ExactRegionProfile {
            width_mm: radius_mm * 2.0,
            depth_mm: radius_mm * 2.0,
            circle: Some(ExactCircleProfile {
                center_x_bits: center_mm[0].to_bits(),
                center_y_bits: center_mm[1].to_bits(),
                radius_bits: radius_mm.to_bits(),
                clockwise: false,
            }),
            mixed_profile: None,
            frame: transform,
        }),
    }
}

fn exact_mixed_profile_from_solved(
    profile: &SolvedSketchRegionProfile,
) -> Option<ExactMixedProfile> {
    let segments = match profile {
        SolvedSketchRegionProfile::Polyline(points) => points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
            .map(|(start_mm, end_mm)| ProfileSegment::Line {
                start_mm: *start_mm,
                end_mm: *end_mm,
            })
            .collect::<Vec<_>>(),
        SolvedSketchRegionProfile::Boundary(edges) => edges
            .iter()
            .map(|edge| match edge {
                SolvedSketchRegionEdge::Line { start_mm, end_mm } => Some(ProfileSegment::Line {
                    start_mm: *start_mm,
                    end_mm: *end_mm,
                }),
                SolvedSketchRegionEdge::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => Some(ProfileSegment::CircularArc {
                    start_mm: *start_mm,
                    end_mm: *end_mm,
                    center_mm: *center_mm,
                    clockwise: *clockwise,
                }),
                SolvedSketchRegionEdge::CubicBezier { .. } => None,
            })
            .collect::<Option<Vec<_>>>()?,
        SolvedSketchRegionProfile::Circle { .. } => return None,
    };
    exact_mixed_profile(&segments, true)
}

fn append_exact_mixed_profile_identity(
    input: &mut String,
    label: &str,
    profile: &ExactMixedProfile,
) {
    write!(
        input,
        ":{label}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
        profile.bounds_bits[0],
        profile.bounds_bits[1],
        profile.bounds_bits[2],
        profile.bounds_bits[3],
        profile.area_bits,
    )
    .expect("writing to String cannot fail");
    for segment in &profile.segments {
        match segment {
            ExactProfileSegment::Line {
                start_bits,
                end_bits,
            } => write!(
                input,
                ":L:{:016x}:{:016x}:{:016x}:{:016x}",
                start_bits[0], start_bits[1], end_bits[0], end_bits[1],
            ),
            ExactProfileSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => write!(
                input,
                ":A:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                start_bits[0],
                start_bits[1],
                end_bits[0],
                end_bits[1],
                center_bits[0],
                center_bits[1],
                clockwise,
            ),
        }
        .expect("writing to String cannot fail");
    }
}

fn parallel_frame_axes(left: &WorkplaneSpec, right: &WorkplaneSpec) -> bool {
    left.frame.x_axis == right.frame.x_axis
        && left.frame.y_axis == right.frame.y_axis
        && left.frame.normal == right.frame.normal
}

fn line_rectangle_bounds(segments: &[ProfileSegment]) -> Option<[f64; 4]> {
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

fn rectangle_bounds(points: &[[f64; 2]]) -> Option<[f64; 4]> {
    if points.len() != 4 || points.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    let min_x = points.iter().map(|point| point[0]).reduce(f64::min)?;
    let min_y = points.iter().map(|point| point[1]).reduce(f64::min)?;
    let max_x = points.iter().map(|point| point[0]).reduce(f64::max)?;
    let max_y = points.iter().map(|point| point[1]).reduce(f64::max)?;
    if min_x >= max_x || min_y >= max_y {
        return None;
    }
    let mut corners = points.to_vec();
    corners.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then_with(|| left[1].total_cmp(&right[1]))
    });
    (corners
        == vec![
            [min_x, min_y],
            [min_x, max_y],
            [max_x, min_y],
            [max_x, max_y],
        ])
    .then_some([min_x, min_y, max_x, max_y])
}

fn origin_rectangle_size(points: &[[f64; 2]]) -> Option<(f64, f64)> {
    if points.len() != 4 || points.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    let width = points.iter().map(|point| point[0]).reduce(f64::max)?;
    let depth = points.iter().map(|point| point[1]).reduce(f64::max)?;
    if width <= 0.0
        || depth <= 0.0
        || points.iter().any(|point| {
            !matches!(point[0], 0.0) && point[0] != width
                || !matches!(point[1], 0.0) && point[1] != depth
        })
    {
        return None;
    }
    let mut corners = points.to_vec();
    corners.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then_with(|| left[1].total_cmp(&right[1]))
    });
    (corners == vec![[0.0, 0.0], [0.0, depth], [width, 0.0], [width, depth]])
        .then_some((width, depth))
}

fn reference_lineage_digest(reference: &BodySubshapeRef) -> String {
    canonical_reference_lineage_digest(
        reference.document_id,
        reference.producer_feature_id,
        &reference.semantic_role,
        &reference.source_element_id,
        &reference.expected_type,
    )
}

#[must_use]
pub fn canonical_reference_lineage_digest(
    document_id: DocumentId,
    producer_feature_id: FeatureId,
    semantic_role: &str,
    source_element_id: &str,
    expected_type: &str,
) -> String {
    let identity = format!(
        "{}:{}:{}:{}:{}",
        document_id.0, producer_feature_id.0, semantic_role, source_element_id, expected_type
    );
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in identity.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
