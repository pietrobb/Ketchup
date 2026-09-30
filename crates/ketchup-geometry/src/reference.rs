//! References to one face or edge of an evaluated body, by the role the evaluator gave it.

use crate::id::{DefinitionId, DocumentId, FeatureId};

pub const BODY_SUBSHAPE_REF_SCHEMA_V1: &str = "ketchup.body-subshape-ref.v1";

/// Well-known faces of an extrusion, as the exact evaluator names them. A
/// result may name other faces too (caps of revolves, sweeps and lofts,
/// offsets, surfaces); those references are valid without a role here.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExactFaceRole {
    Top,
    Bottom,
    /// The side swept by the profile's first line (the profile has no arc).
    LinearSide,
    /// The side swept by the profile's first arc.
    ArcSide,
    /// The side swept by a circular profile.
    CircleSide,
}

const EXACT_FACE_ROLES: [ExactFaceRole; 5] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::LinearSide,
    ExactFaceRole::ArcSide,
    ExactFaceRole::CircleSide,
];

impl ExactFaceRole {
    #[must_use]
    pub const fn semantic_role(self) -> &'static str {
        match self {
            Self::Top => "extrusion.top",
            Self::Bottom => "extrusion.bottom",
            Self::LinearSide => "extrusion.side(profile_edge=line.0)",
            Self::ArcSide => "extrusion.side(profile_edge=arc.0)",
            Self::CircleSide => "extrusion.side(profile_edge=circle)",
        }
    }

    #[must_use]
    pub const fn source_element_id(self) -> &'static str {
        match self {
            Self::Top | Self::Bottom => "profile.face",
            Self::LinearSide => "profile.edge.line.0",
            Self::ArcSide => "profile.edge.arc.0",
            Self::CircleSide => "profile.edge.circle",
        }
    }

    #[must_use]
    pub const fn expected_type(self) -> &'static str {
        match self {
            Self::Top | Self::Bottom | Self::LinearSide => "planar_face",
            Self::CircleSide => "cylindrical_face",
            Self::ArcSide => "face",
        }
    }
}

/// Subshape types a reference may name. Older files also stored edge
/// references; they still load and resolve like any other lost reference.
const SUBSHAPE_REFERENCE_TYPES: [&str; 4] = ["planar_face", "cylindrical_face", "face", "edge"];

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ReferenceStability {
    Guaranteed,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BodySubshapeRef {
    #[serde(serialize_with = "crate::derived::derived")]
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
    // The evaluation that last resolved the reference; a later evaluation replaces it in place.
    #[serde(serialize_with = "crate::derived::derived")]
    pub canonical_input_digest: String,
    #[serde(serialize_with = "crate::derived::derived")]
    pub exact_input_digest: String,
    #[serde(serialize_with = "crate::derived::derived")]
    pub result_fingerprint: String,
    #[serde(serialize_with = "crate::derived::derived")]
    pub evaluator: String,
    #[serde(serialize_with = "crate::derived::derived")]
    pub backend: String,
    #[serde(serialize_with = "crate::derived::derived")]
    pub tolerance: String,
    pub lineage_digest: String,
    #[serde(serialize_with = "crate::derived::derived")]
    pub corroborating_geometry_fingerprint: String,
}

impl BodySubshapeRef {
    #[must_use]
    pub fn role(&self) -> Option<ExactFaceRole> {
        EXACT_FACE_ROLES.into_iter().find(|role| {
            self.semantic_role == role.semantic_role()
                && self.source_element_id == role.source_element_id()
        })
    }

    #[must_use]
    pub fn has_valid_lineage(&self) -> bool {
        self.schema == BODY_SUBSHAPE_REF_SCHEMA_V1
            && self.expected_cardinality == 1
            && !self.semantic_role.is_empty()
            && !self.source_element_id.is_empty()
            && SUBSHAPE_REFERENCE_TYPES.contains(&self.expected_type.as_str())
            && self
                .role()
                .is_none_or(|role| self.expected_type == role.expected_type())
            && self.lineage_digest == reference_lineage_digest(self)
    }
}

/// The lineage digest `reference` must carry.
#[must_use]
pub fn reference_lineage_digest(reference: &BodySubshapeRef) -> String {
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
