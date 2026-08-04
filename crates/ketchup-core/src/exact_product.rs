#![forbid(unsafe_code)]

use crate::document::{DefinitionId, DocumentId, FeatureId, FeatureKind, InstancePath, Snapshot};
use sha2::{Digest, Sha256};
use std::fmt;

pub const EXACT_PRODUCT_SCHEMA_V1: &str = "ketchup.exact-product.v1";
pub const EXACT_RECTANGLE_EVALUATOR_V1: &str = "ketchup.exact-rectangle-evaluator.v1";
pub const EXACT_THROUGH_CUT_EVALUATOR_V1: &str = "ketchup.exact-through-cut-evaluator.v1";
pub const BODY_SUBSHAPE_REF_SCHEMA_V1: &str = "ketchup.body-subshape-ref.v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExactFaceRole {
    Top,
    Bottom,
    East,
    CutWest,
    CutEast,
    CutSouth,
    CutNorth,
}

const EXTRUSION_FACE_ROLES: [ExactFaceRole; 3] = [
    ExactFaceRole::Top,
    ExactFaceRole::Bottom,
    ExactFaceRole::East,
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

impl ExactFaceRole {
    #[must_use]
    pub const fn semantic_role(self) -> &'static str {
        match self {
            Self::Top => "extrusion.top",
            Self::Bottom => "extrusion.bottom",
            Self::East => "extrusion.side(profile_edge=east)",
            Self::CutWest => "through_cut.wall.west",
            Self::CutEast => "through_cut.wall.east",
            Self::CutSouth => "through_cut.wall.south",
            Self::CutNorth => "through_cut.wall.north",
        }
    }

    #[must_use]
    pub const fn source_element_id(self) -> &'static str {
        match self {
            Self::Top | Self::Bottom => "profile.face",
            Self::East => "profile.edge.east",
            Self::CutWest => "cut_profile.edge.west",
            Self::CutEast => "cut_profile.edge.east",
            Self::CutSouth => "cut_profile.edge.south",
            Self::CutNorth => "cut_profile.edge.north",
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
            ExactFaceRole::CutWest,
            ExactFaceRole::CutEast,
            ExactFaceRole::CutSouth,
            ExactFaceRole::CutNorth,
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

    #[must_use]
    pub fn matches_request(&self, request: &ExactRectangleRequest) -> bool {
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

#[derive(Clone, Debug, Eq, PartialEq)]
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
            && ExactRectangleRequest::from_snapshot(snapshot, self.identity.definition_id)
                .is_ok_and(|request| self.validate_for_request(&request).is_ok())
    }

    pub fn validate_for_request(
        &self,
        request: &ExactRectangleRequest,
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
        let expected_counts = if request.through_cut.is_some() {
            (16, 32)
        } else {
            (8, 12)
        };
        let expected_triangle_roles = expected_roles.iter().all(|role| {
            let expected = match role {
                ExactFaceRole::Top | ExactFaceRole::Bottom if request.through_cut.is_some() => 8,
                _ => 2,
            };
            self.triangles
                .iter()
                .filter(|triangle| triangle.face_role == Some(*role))
                .count()
                == expected
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactThroughCutRequest {
    pub feature_id: FeatureId,
    pub profile_feature_id: FeatureId,
    pub min_x_bits: u64,
    pub min_y_bits: u64,
    pub width_bits: u64,
    pub depth_bits: u64,
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
    pub through_cut: Option<ExactThroughCutRequest>,
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
                    FeatureKind::Profile { .. } | FeatureKind::ThroughCut { .. } => None,
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
        let cuts = definition
            .feature_ids()
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                match feature.kind() {
                    FeatureKind::ThroughCut { target, profile } => Some((*id, *target, *profile)),
                    FeatureKind::Profile { .. } | FeatureKind::Extrusion { .. } => None,
                }
            })
            .collect::<Vec<_>>();
        let through_cut = match cuts.as_slice() {
            [] => None,
            [(feature_id, target, cut_profile_id)] if target == extrusion_feature_id => {
                let cut_profile = snapshot
                    .feature(*cut_profile_id)
                    .ok_or(ExactProductError::ProfileNotFound(*cut_profile_id))?;
                let FeatureKind::Profile {
                    points_mm: cut_points,
                } = cut_profile.kind()
                else {
                    return Err(ExactProductError::UnsupportedProfile);
                };
                let [min_x, min_y, max_x, max_y] =
                    rectangle_bounds(cut_points).ok_or(ExactProductError::UnsupportedThroughCut)?;
                if min_x <= 0.0 || min_y <= 0.0 || max_x >= width_mm || max_y >= depth_mm {
                    return Err(ExactProductError::UnsupportedThroughCut);
                }
                Some(ExactThroughCutRequest {
                    feature_id: *feature_id,
                    profile_feature_id: *cut_profile_id,
                    min_x_bits: min_x.to_bits(),
                    min_y_bits: min_y.to_bits(),
                    width_bits: (max_x - min_x).to_bits(),
                    depth_bits: (max_y - min_y).to_bits(),
                })
            }
            _ => return Err(ExactProductError::UnsupportedDefinition),
        };
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
        let canonical_input_digest = through_cut.as_ref().map_or_else(
            || digest(&canonical_input),
            |cut| {
                digest(&format!(
                    "{canonical_input}:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}",
                    cut.feature_id.0,
                    cut.profile_feature_id.0,
                    cut.min_x_bits,
                    cut.min_y_bits,
                    cut.width_bits,
                    cut.depth_bits
                ))
            },
        );
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
            through_cut,
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
    pub fn producer_feature_id(&self) -> FeatureId {
        self.through_cut
            .as_ref()
            .map_or(self.extrusion_feature_id, |cut| cut.feature_id)
    }

    #[must_use]
    pub fn evaluator(&self) -> &'static str {
        if self.through_cut.is_some() {
            EXACT_THROUGH_CUT_EVALUATOR_V1
        } else {
            EXACT_RECTANGLE_EVALUATOR_V1
        }
    }

    #[must_use]
    pub fn profile_feature_id_for_role(&self, role: ExactFaceRole) -> Option<FeatureId> {
        match role {
            ExactFaceRole::Top | ExactFaceRole::Bottom | ExactFaceRole::East => {
                Some(self.profile_feature_id)
            }
            ExactFaceRole::CutWest
            | ExactFaceRole::CutEast
            | ExactFaceRole::CutSouth
            | ExactFaceRole::CutNorth => {
                self.through_cut.as_ref().map(|cut| cut.profile_feature_id)
            }
        }
    }

    fn expected_face_roles(&self) -> &'static [ExactFaceRole] {
        if self.through_cut.is_some() {
            &THROUGH_CUT_FACE_ROLES
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
            Self::UnsupportedThroughCut => formatter.write_str(
                "exact M3 supports one strictly bounded axis-aligned rectangle through-cut",
            ),
            Self::InvalidWorkerEvidence => {
                formatter.write_str("exact worker evidence does not match the canonical request")
            }
        }
    }
}

impl std::error::Error for ExactProductError {}

pub fn build_box_render_package<const N: usize>(
    request: &ExactRectangleRequest,
    exact_input_digest: String,
    result_fingerprint: String,
    backend: String,
    tolerance: String,
    worker_bounds_mm: [[f64; 3]; 2],
    face_evidence: [(ExactFaceRole, String, String); N],
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
        || !valid_evidence_roles(request, &face_evidence)
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
                    expected_type: "planar_face".to_owned(),
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
    request: &ExactRectangleRequest,
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
                    "planar_face",
                )
        })
    {
        return false;
    }
    let expected_roles = request.expected_face_roles();
    roles.len() == expected_roles.len() && roles == expected_roles.to_vec()
}

fn render_mesh(
    request: &ExactRectangleRequest,
) -> Result<(Vec<ExactVertex>, Vec<ExactTriangle>), ExactProductError> {
    let [width, depth, height] = request.dimensions_mm();
    if [width, depth, height]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0)
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    let outer = [
        [0.0, 0.0, 0.0],
        [width, 0.0, 0.0],
        [width, depth, 0.0],
        [0.0, depth, 0.0],
        [0.0, 0.0, height],
        [width, 0.0, height],
        [width, depth, height],
        [0.0, depth, height],
    ];
    let Some(cut) = &request.through_cut else {
        let vertices = outer
            .map(|position_mm| ExactVertex { position_mm })
            .to_vec();
        let triangles = [
            ([0, 2, 1], Some(ExactFaceRole::Bottom)),
            ([0, 3, 2], Some(ExactFaceRole::Bottom)),
            ([4, 5, 6], Some(ExactFaceRole::Top)),
            ([4, 6, 7], Some(ExactFaceRole::Top)),
            ([1, 2, 6], Some(ExactFaceRole::East)),
            ([1, 6, 5], Some(ExactFaceRole::East)),
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
    let mut positions = outer.to_vec();
    positions.extend([
        [x0, y0, 0.0],
        [x1, y0, 0.0],
        [x1, y1, 0.0],
        [x0, y1, 0.0],
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
