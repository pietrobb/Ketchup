#![forbid(unsafe_code)]

use crate::beam_m5::{BeamExactPiecePackage, BeamExactResultKey};
use crate::document::{
    BooleanOperation, BottleEdgeFinishKind, CanonicalCommand, CommandBatch, DefinitionId,
    DocumentId, ExactReferenceConversionConsequence, ExactToMeshConversion, FeatureId, FeatureKind,
    InstancePath, MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec, ProfileSegment, Snapshot,
    Transform,
};
use crate::graph::DerivedIdentity;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

pub const EXACT_PRODUCT_SCHEMA_V1: &str = "ketchup.exact-product.v1";
pub const EXACT_RECTANGLE_EVALUATOR_V1: &str = "ketchup.exact-rectangle-evaluator.v1";
pub const EXACT_CIRCLE_EVALUATOR_V1: &str = "ketchup.exact-circle-evaluator.v1";
pub const EXACT_ARC_PROFILE_EVALUATOR_V1: &str = "ketchup.exact-arc-profile-evaluator.v1";
pub const EXACT_THROUGH_CUT_EVALUATOR_V1: &str = "ketchup.exact-through-cut-evaluator.v1";
pub const EXACT_CIRCULAR_CUT_EVALUATOR_V1: &str = "ketchup.exact-circular-cut-evaluator.v1";
pub const EXACT_POCKET_EVALUATOR_V1: &str = "ketchup.exact-pocket-evaluator.v1";
pub const EXACT_BOOLEAN_UNION_EVALUATOR_V1: &str = "ketchup.exact-boolean-union-evaluator.v1";
pub const EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1: &str =
    "ketchup.exact-boolean-intersect-evaluator.v1";
pub const EXACT_BOOLEAN_SPLIT_EVALUATOR_V1: &str = "ketchup.exact-boolean-split-evaluator.v1";
pub const EXACT_PLANAR_OFFSET_SCHEMA_V1: &str = "ketchup.exact-planar-offset.v1";
pub const EXACT_PLANAR_OFFSET_EVALUATOR_V1: &str = "ketchup.exact-planar-offset-evaluator.v1";
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
    CircleSide,
    ArcSide,
    CutCircle,
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
const CIRCULAR_CUT_FACE_ROLES: [ExactFaceRole; 4] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
    ExactFaceRole::CutCircle,
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
            Self::CircleSide => "extrusion.side(profile_edge=circle)",
            Self::ArcSide => "extrusion.side(profile_edge=arc.0)",
            Self::CutCircle => "through_cut.wall.circle",
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
            Self::CircleSide => "profile.edge.circle",
            Self::ArcSide => "profile.edge.arc.0",
            Self::CutCircle => "cut_profile.edge.circle",
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
            ExactFaceRole::CircleSide,
            ExactFaceRole::ArcSide,
            ExactFaceRole::CutCircle,
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
    pub fn has_valid_lineage(&self) -> bool {
        self.schema == BODY_SUBSHAPE_REF_SCHEMA_V1
            && self.expected_cardinality == 1
            && self
                .role()
                .is_some_and(|role| self.expected_type == role.expected_type())
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
    pub fn matches_request(&self, request: &ExactFeatureChainRequest) -> bool {
        let Some(role) = self.role() else {
            return false;
        };
        self.has_valid_lineage()
            && self.document_id == request.document_id
            && self.definition_id == request.definition_id
            && request.profile_feature_id_for_role(role) == Some(self.profile_feature_id)
            && self.producer_feature_id == request.producer_feature_id()
            && self.canonical_input_digest == request.canonical_input_digest
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
pub enum ExactBodyPackage {
    Rectangle(ExactRenderPackage),
    Revolve(crate::bottle_m6::ExactRevolvePackage),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactMeshExport {
    pub mesh_obj: String,
    pub loss_report: String,
}

pub trait ExactBodyView {
    fn bounds_mm(&self) -> [[f64; 3]; 2];
    fn vertex_count(&self) -> usize;
    fn vertex_position_mm(&self, index: usize) -> [f64; 3];
    fn triangle_count(&self) -> usize;
    fn triangle_indices(&self, index: usize) -> [u32; 3];
    fn triangle_group(&self, index: usize) -> &'static str;
    fn tolerance(&self) -> &str;
    fn source_digest(&self) -> &str;
    fn producer_identity(&self) -> String;
    fn result_fingerprint(&self) -> &str;

    #[must_use]
    fn mesh_export(&self, transform: Transform) -> ExactMeshExport {
        mesh_export_from_view(self, transform)
    }
}

impl ExactBodyPackage {
    #[must_use]
    pub fn definition_id(&self) -> DefinitionId {
        match self {
            Self::Rectangle(package) => package.identity.definition_id,
            Self::Revolve(package) => package.identity.definition_id,
        }
    }

    #[must_use]
    pub fn producer_feature_id(&self) -> FeatureId {
        match self {
            Self::Rectangle(package) => package.identity.producer_feature_id,
            Self::Revolve(package) => package.identity.producer_feature_id,
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
        }
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        match self {
            Self::Rectangle(package) => package.is_current(snapshot),
            Self::Revolve(package) => package.is_current(snapshot),
        }
    }

    #[must_use]
    pub fn bounds_mm(&self) -> [[f64; 3]; 2] {
        match self {
            Self::Rectangle(package) => package.bounds_mm,
            Self::Revolve(package) => package.bounds_mm,
        }
    }

    #[must_use]
    pub fn vertices(&self) -> &[ExactVertex] {
        match self {
            Self::Rectangle(package) => &package.vertices,
            Self::Revolve(package) => &package.vertices,
        }
    }

    #[must_use]
    pub fn triangles(&self) -> &[ExactTriangle] {
        match self {
            Self::Rectangle(package) => &package.triangles,
            Self::Revolve(package) => &package.triangles,
        }
    }

    #[must_use]
    pub fn references(&self) -> &[BodySubshapeRef] {
        match self {
            Self::Rectangle(package) => &package.references,
            Self::Revolve(package) => &package.references,
        }
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
    pub fn revolve(&self) -> Option<&crate::bottle_m6::ExactRevolvePackage> {
        match self {
            Self::Revolve(package) => Some(package),
            Self::Rectangle(_) => None,
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

    fn tolerance(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.tolerance,
            Self::Revolve(package) => &package.identity.tolerance,
        }
    }

    fn source_digest(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.source_digest,
            Self::Revolve(package) => &package.identity.source_digest,
        }
    }

    fn producer_identity(&self) -> String {
        format!("producer_feature_id={}", self.producer_feature_id().0)
    }

    fn result_fingerprint(&self) -> &str {
        match self {
            Self::Rectangle(package) => &package.identity.result_fingerprint,
            Self::Revolve(package) => &package.identity.result_fingerprint,
        }
    }
}

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
        let group = view.triangle_group(triangle_index);
        if current_group != Some(group) {
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

impl From<crate::bottle_m6::ExactRevolvePackage> for ExactBodyPackage {
    fn from(package: crate::bottle_m6::ExactRevolvePackage) -> Self {
        Self::Revolve(package)
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

#[derive(Clone, Debug, Default)]
pub struct ExactResultRegistry {
    packages: BTreeMap<ExactResultKey, Arc<ExactBodyPackage>>,
    beam_packages: BTreeMap<BeamExactResultKey, Arc<BeamExactPiecePackage>>,
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
        Ok(())
    }

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
        Ok(())
    }

    #[must_use]
    pub fn get_result(&self, key: &ExactResultKey) -> Option<&Arc<ExactBodyPackage>> {
        self.packages.get(key)
    }

    #[must_use]
    pub fn get_beam_result(&self, key: &BeamExactResultKey) -> Option<&Arc<BeamExactPiecePackage>> {
        self.beam_packages.get(key)
    }

    #[must_use]
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

    pub fn beam_values(&self) -> impl Iterator<Item = &Arc<BeamExactPiecePackage>> {
        self.beam_packages.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.packages.len()
    }

    #[must_use]
    pub fn beam_len(&self) -> usize {
        self.beam_packages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub fn clear(&mut self) {
        self.packages.clear();
        self.beam_packages.clear();
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
            && ExactFeatureChainRequest::from_snapshot(snapshot, self.identity.definition_id)
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
        let is_circle = request.circle.is_some();
        let is_mixed = request.mixed_profile.is_some();
        let mixed_mesh = is_mixed
            .then(|| render_mesh(request))
            .transpose()?
            .map(|(vertices, triangles)| (vertices.len(), triangles));
        let is_circular_cut = request
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.circle.is_some());
        let expected_counts = if is_circular_cut {
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
            if is_circular_cut {
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactBoxShellRequest {
    pub shell_feature_id: FeatureId,
    pub thickness_bits: u64,
    pub edge_finish_feature_id: Option<FeatureId>,
    pub edge_finish_kind: Option<BottleEdgeFinishKind>,
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
            || distance_mm.abs() <= 1.0e-6
            || output_bounds.into_iter().any(|value| !value.is_finite())
            || output_bounds[2] - output_bounds[0] <= 1.0e-6
            || output_bounds[3] - output_bounds[1] <= 1.0e-6
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
            || sections
                .iter()
                .any(|section| !(4..=64).contains(&section.control_point_bits.len()))
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
    pub boolean: Option<ExactBooleanRequest>,
    pub shell: Option<ExactBoxShellRequest>,
    pub canonical_input_digest: String,
}

impl ExactFeatureChainRequest {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        if definition.feature_ids().iter().any(|feature_id| {
            snapshot.feature(*feature_id).is_some_and(|feature| {
                matches!(
                    feature.kind(),
                    FeatureKind::BottleProfileControl { .. } | FeatureKind::Revolve { .. }
                )
            })
        }) {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let extrusions = definition
            .feature_ids()
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                let FeatureKind::Extrusion { profile, height } = feature.kind() else {
                    return None;
                };
                Some((*id, *profile, height.millimetres()))
            })
            .collect::<Vec<_>>();
        let legacy_cuts = definition
            .feature_ids()
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
        let pockets = definition
            .feature_ids()
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
        let booleans = definition
            .feature_ids()
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
        let shells = definition
            .feature_ids()
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
        let finishes = definition
            .feature_ids()
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
                    let ([min_x, min_y, max_x, max_y], tool_circle) = match tool_profile.kind() {
                        FeatureKind::Profile { points_mm } => (
                            rectangle_bounds(points_mm)
                                .ok_or(ExactProductError::UnsupportedBoolean(operation))?,
                            None,
                        ),
                        FeatureKind::SegmentProfile { segments, closed } => {
                            let circle = exact_circle_profile(segments, *closed)
                                .ok_or(ExactProductError::UnsupportedBoolean(operation))?;
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
                            )
                        }
                        _ => return Err(ExactProductError::UnsupportedProfile),
                    };
                    let supported = match operation {
                        BooleanOperation::Cut => {
                            min_x > 0.0 && min_y > 0.0 && max_x < width_mm && max_y < depth_mm
                        }
                        BooleanOperation::Union if tool_circle.is_none() => {
                            rectangular_union_bounds(
                                width_mm,
                                depth_mm,
                                [min_x, min_y, max_x, max_y],
                            )
                            .is_some()
                        }
                        BooleanOperation::Union => false,
                        BooleanOperation::Intersect if tool_circle.is_none() => {
                            rectangular_intersection_bounds(
                                width_mm,
                                depth_mm,
                                [min_x, min_y, max_x, max_y],
                            )
                            .is_some()
                        }
                        BooleanOperation::Intersect => false,
                        BooleanOperation::Split if tool_circle.is_none() => {
                            rectangular_split_supported(
                                width_mm,
                                depth_mm,
                                [min_x, min_y, max_x, max_y],
                            )
                        }
                        BooleanOperation::Split => false,
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
                    })
                },
            )
            .transpose()?;
        let source_digest = snapshot.canonical_digest();
        let canonical_input = format!(
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
        let mut canonical_input_digest = boolean.as_ref().map_or_else(
            || digest(&canonical_input),
            |cut| {
                if legacy_through_cut {
                    digest(&format!(
                        "{canonical_input}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}",
                        cut.feature_id.0,
                        cut.profile_feature_id.0,
                        cut.min_x_bits,
                        cut.min_y_bits,
                        cut.width_bits,
                        cut.depth_bits
                    ))
                } else {
                    digest(&format!(
                        "{canonical_input}:{}:{}:{}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}",
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
        );
        if let Some(circle) = circle {
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:circle:{:016x}:{:016x}:{:016x}:{}",
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
        }
        if let Some(tool_circle) = boolean.as_ref().and_then(|boolean| boolean.circle) {
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:cut-circle:{:016x}:{:016x}:{:016x}:{}",
                tool_circle.center_x_bits,
                tool_circle.center_y_bits,
                tool_circle.radius_bits,
                tool_circle.clockwise
            ));
        }
        let pocket_depth_bits = pocket_depth_mm.map(f64::to_bits);
        if let Some(depth_bits) = pocket_depth_bits {
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:pocket:{depth_bits:016x}"
            ));
        }
        if let Some(shell) = &shell {
            canonical_input_digest = digest(&format!(
                "{canonical_input_digest}:shell:{}:{:016x}:finish:{}:{}:{:016x}",
                shell.shell_feature_id.0,
                shell.thickness_bits,
                shell.edge_finish_feature_id.map_or(0, |id| id.0),
                shell.edge_finish_kind.map_or("none", |kind| match kind {
                    BottleEdgeFinishKind::Fillet => "fillet",
                    BottleEdgeFinishKind::Chamfer => "chamfer",
                }),
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
            boolean,
            shell,
            canonical_input_digest,
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
            Some(BooleanOperation::Cut)
                if self
                    .boolean
                    .as_ref()
                    .is_some_and(|boolean| boolean.circle.is_some()) =>
            {
                EXACT_CIRCULAR_CUT_EVALUATOR_V1
            }
            Some(BooleanOperation::Cut) if self.pocket_depth_bits.is_some() => {
                EXACT_POCKET_EVALUATOR_V1
            }
            Some(BooleanOperation::Cut) => EXACT_THROUGH_CUT_EVALUATOR_V1,
            Some(BooleanOperation::Union) => EXACT_BOOLEAN_UNION_EVALUATOR_V1,
            Some(BooleanOperation::Intersect) => EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1,
            Some(BooleanOperation::Split) => EXACT_BOOLEAN_SPLIT_EVALUATOR_V1,
            None if self.circle.is_some() => EXACT_CIRCLE_EVALUATOR_V1,
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
            | ExactFaceRole::CircleSide
            | ExactFaceRole::ArcSide => Some(self.profile_feature_id),
            ExactFaceRole::CutCircle => self.boolean.as_ref().map(|cut| cut.profile_feature_id),
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

    fn expected_face_roles(&self) -> &'static [ExactFaceRole] {
        if self.shell.is_some() {
            return &BOX_SHELL_FACE_ROLES;
        }
        if self
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.circle.is_some())
        {
            &CIRCULAR_CUT_FACE_ROLES
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
    InvalidWorkerEvidence,
    StaleResult,
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
            Self::InvalidWorkerEvidence => {
                formatter.write_str("exact worker evidence does not match the canonical request")
            }
            Self::StaleResult => formatter.write_str("exact result is stale for the snapshot"),
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
    let expected_lineage = canonical_reference_lineage_digest(
        request.document_id,
        request.offset_feature_id,
        ExactFaceRole::PlanarOffsetFace.semantic_role(),
        ExactFaceRole::PlanarOffsetFace.source_element_id(),
        ExactFaceRole::PlanarOffsetFace.expected_type(),
    );
    if worker_bounds_mm != expected_bounds
        || !worker_area_mm2.is_finite()
        || (worker_area_mm2 - request.expected_area_mm2()).abs() > 1.0e-6
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
    let min = expected_min;
    let max = expected_max;
    let (vertices, triangles) = render_mesh(request)?;
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
    if let Some(circle) = request.circle {
        return render_circle_mesh(circle, height);
    }
    if let Some(mixed) = &request.mixed_profile {
        return render_mixed_profile_mesh(mixed, height);
    }
    if let Some(circle) = request.boolean.as_ref().and_then(|boolean| boolean.circle) {
        return render_circular_cut_mesh(width, depth, height, circle);
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
        .filter(|boolean| boolean.operation == BooleanOperation::Cut)
    else {
        let vertices = outer
            .map(|position_mm| ExactVertex { position_mm })
            .to_vec();
        let (bottom_role, top_role, east_role) = if request.shell.is_some() {
            (
                ExactFaceRole::BoxShellOuterBottom,
                ExactFaceRole::BoxShellRim,
                ExactFaceRole::BoxShellOuterEast,
            )
        } else {
            (
                ExactFaceRole::Bottom,
                ExactFaceRole::Top,
                ExactFaceRole::East,
            )
        };
        let triangles = [
            ([0, 2, 1], Some(bottom_role)),
            ([0, 3, 2], Some(bottom_role)),
            ([4, 5, 6], Some(top_role)),
            ([4, 6, 7], Some(top_role)),
            ([1, 2, 6], Some(east_role)),
            ([1, 6, 5], Some(east_role)),
            ([0, 4, 7], None),
            ([0, 7, 3], None),
            ([3, 7, 6], None),
            ([3, 6, 2], None),
            ([0, 1, 5], None),
            ([0, 5, 4], None),
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

fn render_mixed_profile_mesh(
    profile: &ExactMixedProfile,
    height: f64,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    if !height.is_finite() || height <= 0.0 {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let mut boundary = Vec::<[f64; 2]>::new();
    let mut arc_side_edges = Vec::<bool>::new();
    let first_arc = profile
        .segments
        .iter()
        .position(|segment| matches!(segment, ExactProfileSegment::CircularArc { .. }))
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
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
        } else if boundary.last() != Some(&start) {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        for step in 1..=steps {
            let point = if sweep == 0.0 {
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
            arc_side_edges.push(segment_index == first_arc);
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
    if boundary.len() < 3 || arc_side_edges.len() != boundary.len() {
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
    for (index, arc_side) in arc_side_edges.iter().copied().enumerate() {
        let next = (index + 1) % boundary.len();
        let bottom = (index * 2) as u32;
        let top = bottom + 1;
        let next_bottom = (next * 2) as u32;
        let next_top = next_bottom + 1;
        let role = arc_side.then_some(ExactFaceRole::ArcSide);
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
        || center_x - radius <= 0.0
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

fn exact_mixed_profile(segments: &[ProfileSegment], closed: bool) -> Option<ExactMixedProfile> {
    if !closed
        || !(2..=64).contains(&segments.len())
        || !segments
            .iter()
            .any(|segment| matches!(segment, ProfileSegment::Line { .. }))
        || !segments
            .iter()
            .any(|segment| matches!(segment, ProfileSegment::CircularArc { .. }))
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
                    || radius <= 0.0
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
    let mut delta = candidate - start;
    if sweep > 0.0 {
        while delta < 0.0 {
            delta += std::f64::consts::TAU;
        }
        delta <= sweep + 1.0e-12
    } else {
        while delta > 0.0 {
            delta -= std::f64::consts::TAU;
        }
        delta >= sweep - 1.0e-12
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
