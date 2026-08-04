#![forbid(unsafe_code)]

use crate::document::{DefinitionId, DocumentId, FeatureId, FeatureKind, InstancePath, Snapshot};
use sha2::{Digest, Sha256};
use std::fmt;

pub const EXACT_PRODUCT_SCHEMA_V1: &str = "ketchup.exact-product.v1";
pub const EXACT_RECTANGLE_EVALUATOR_V1: &str = "ketchup.exact-rectangle-evaluator.v1";
pub const BODY_SUBSHAPE_REF_SCHEMA_V1: &str = "ketchup.body-subshape-ref.v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExactFaceRole {
    Top,
    Bottom,
    East,
}

impl ExactFaceRole {
    #[must_use]
    pub const fn semantic_role(self) -> &'static str {
        match self {
            Self::Top => "extrusion.top",
            Self::Bottom => "extrusion.bottom",
            Self::East => "extrusion.side(profile_edge=east)",
        }
    }

    #[must_use]
    pub const fn source_element_id(self) -> &'static str {
        match self {
            Self::Top | Self::Bottom => "profile.face",
            Self::East => "profile.edge.east",
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
            && self.expected_type == "planar_face"
            && self.expected_cardinality == 1
            && self.role().is_some()
            && self.lineage_digest == reference_lineage_digest(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodyResultIdentity {
    pub schema: String,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub extrusion_feature_id: FeatureId,
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
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssemblySelectionTarget {
    pub instance_path: InstancePath,
    pub body: BodySubshapeRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactRectangleRequest {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: DefinitionId,
    pub profile_feature_id: FeatureId,
    pub extrusion_feature_id: FeatureId,
    pub width_bits: u64,
    pub depth_bits: u64,
    pub height_bits: u64,
    pub canonical_input_digest: String,
}

impl ExactRectangleRequest {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Result<Self, ExactProductError> {
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactProductError::DefinitionNotFound(definition_id))?;
        let extrusions = definition
            .feature_ids()
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                match feature.kind() {
                    FeatureKind::Extrusion { profile, height } => {
                        Some((*id, *profile, height.millimetres()))
                    }
                    FeatureKind::Profile { .. } => None,
                }
            })
            .collect::<Vec<_>>();
        let [(extrusion_feature_id, profile_feature_id, height_mm)] = extrusions.as_slice() else {
            return Err(ExactProductError::UnsupportedDefinition);
        };
        let profile = snapshot
            .feature(*profile_feature_id)
            .ok_or(ExactProductError::ProfileNotFound(*profile_feature_id))?;
        if profile.definition_id() != definition_id {
            return Err(ExactProductError::UnsupportedDefinition);
        }
        let FeatureKind::Profile { points_mm } = profile.kind() else {
            return Err(ExactProductError::UnsupportedProfile);
        };
        let (width_mm, depth_mm) =
            origin_rectangle_size(points_mm).ok_or(ExactProductError::UnsupportedProfile)?;
        if !height_mm.is_finite() || *height_mm <= 0.0 {
            return Err(ExactProductError::UnsupportedExtrusion);
        }
        let source_digest = snapshot.canonical_digest();
        let canonical_input_digest = digest(&format!(
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
        ));
        Ok(Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest,
            definition_id,
            profile_feature_id: *profile_feature_id,
            extrusion_feature_id: *extrusion_feature_id,
            width_bits: width_mm.to_bits(),
            depth_bits: depth_mm.to_bits(),
            height_bits: height_mm.to_bits(),
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactProductError {
    DefinitionNotFound(DefinitionId),
    ProfileNotFound(FeatureId),
    UnsupportedDefinition,
    UnsupportedProfile,
    UnsupportedExtrusion,
    InvalidWorkerEvidence,
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
            Self::InvalidWorkerEvidence => {
                formatter.write_str("exact worker evidence does not match the canonical request")
            }
        }
    }
}

impl std::error::Error for ExactProductError {}

pub fn build_box_render_package(
    request: &ExactRectangleRequest,
    exact_input_digest: String,
    result_fingerprint: String,
    backend: String,
    tolerance: String,
    worker_bounds_mm: [[f64; 3]; 2],
    face_evidence: [(ExactFaceRole, String, String); 3],
) -> Result<ExactRenderPackage, ExactProductError> {
    let [worker_min, worker_max] = worker_bounds_mm;
    let dimensions = request.dimensions_mm();
    if worker_min
        .into_iter()
        .chain(worker_max)
        .any(|value| !value.is_finite())
        || (0..3).any(|axis| {
            worker_min[axis].abs() > 1.0e-6 || (worker_max[axis] - dimensions[axis]).abs() > 1.0e-6
        })
        || exact_input_digest.is_empty()
        || result_fingerprint.is_empty()
        || backend.is_empty()
        || tolerance.is_empty()
        || face_evidence
            .iter()
            .any(|(_, lineage_digest, fingerprint)| {
                lineage_digest.is_empty() || fingerprint.is_empty()
            })
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let min = [0.0; 3];
    let max = dimensions;
    let vertices = [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [max[0], max[1], max[2]],
        [min[0], max[1], max[2]],
    ]
    .map(|position_mm| ExactVertex { position_mm })
    .to_vec();
    let triangles = vec![
        ExactTriangle {
            vertex_indices: [0, 2, 1],
            face_role: Some(ExactFaceRole::Bottom),
        },
        ExactTriangle {
            vertex_indices: [0, 3, 2],
            face_role: Some(ExactFaceRole::Bottom),
        },
        ExactTriangle {
            vertex_indices: [4, 5, 6],
            face_role: Some(ExactFaceRole::Top),
        },
        ExactTriangle {
            vertex_indices: [4, 6, 7],
            face_role: Some(ExactFaceRole::Top),
        },
        ExactTriangle {
            vertex_indices: [1, 2, 6],
            face_role: Some(ExactFaceRole::East),
        },
        ExactTriangle {
            vertex_indices: [1, 6, 5],
            face_role: Some(ExactFaceRole::East),
        },
        ExactTriangle {
            vertex_indices: [0, 4, 7],
            face_role: None,
        },
        ExactTriangle {
            vertex_indices: [0, 7, 3],
            face_role: None,
        },
        ExactTriangle {
            vertex_indices: [3, 7, 6],
            face_role: None,
        },
        ExactTriangle {
            vertex_indices: [3, 6, 2],
            face_role: None,
        },
        ExactTriangle {
            vertex_indices: [0, 1, 5],
            face_role: None,
        },
        ExactTriangle {
            vertex_indices: [0, 5, 4],
            face_role: None,
        },
    ];
    let references = face_evidence
        .into_iter()
        .map(
            |(role, lineage_digest, corroborating_geometry_fingerprint)| BodySubshapeRef {
                schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
                document_id: request.document_id,
                definition_id: request.definition_id,
                profile_feature_id: request.profile_feature_id,
                producer_feature_id: request.extrusion_feature_id,
                semantic_role: role.semantic_role().to_owned(),
                source_element_id: role.source_element_id().to_owned(),
                expected_type: "planar_face".to_owned(),
                expected_cardinality: 1,
                stability: ReferenceStability::Guaranteed,
                canonical_input_digest: request.canonical_input_digest.clone(),
                exact_input_digest: exact_input_digest.clone(),
                result_fingerprint: result_fingerprint.clone(),
                evaluator: EXACT_RECTANGLE_EVALUATOR_V1.to_owned(),
                backend: backend.clone(),
                tolerance: tolerance.clone(),
                lineage_digest,
                corroborating_geometry_fingerprint,
            },
        )
        .collect::<Vec<_>>();
    if references.len() != 3
        || references
            .iter()
            .any(|reference| !reference.has_valid_lineage())
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
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest,
            result_fingerprint,
            evaluator: EXACT_RECTANGLE_EVALUATOR_V1.to_owned(),
            backend,
            tolerance,
        },
        bounds_mm: [min, max],
        vertices,
        triangles,
        references,
    })
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
