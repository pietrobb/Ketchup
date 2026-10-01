#![forbid(unsafe_code)]

use crate::assembly::{AxialAttachment, AxialAttachmentKind, PlanarFaceAttachment};
use crate::document::{
    BodyId, BooleanOperation, DefinitionId, DocumentId, FeatureDependencyGraph, FeatureId,
    FeatureKind, ImportedExactBodySpec, InstancePath, MeshBodySpec, ProfileSegment, Snapshot,
    Transform,
};
use crate::exact_brep_graph::{
    ExactBRepGraph, ExactBRepGraphError, ExactBRepOperation, ExactBRepPlanarLoop,
    ExactBRepPlanarSegment,
};
use crate::graph::{DerivedIdentity, sha256_hex};
use crate::import::StepImportMesh;
use crate::tolerance::{APPROXIMATION, MAX_COORDINATE_MM, ROUNDING, TolerancePolicy};
use crate::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceError,
    TopologicalReferenceResolution, TopologicalReferenceStability,
    canonical_topological_lineage_digest, publish_generated_topological_references,
    publish_imported_topological_references,
    resolve_topological_reference as resolve_role_neutral_topological_reference,
    topological_edge_provenance_tokens,
};
use ketchup_geometry::sketch::{
    SolvedSketchRegion, SolvedSketchRegionEdge, SolvedSketchRegionProfile,
};
use ketchup_tolerance::limits;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use ketchup_geometry::linalg::CubicBezier;
use ketchup_geometry::reference::reference_lineage_digest;
pub use ketchup_geometry::reference::{
    BODY_SUBSHAPE_REF_SCHEMA_V1, BodySubshapeRef, ExactFaceRole, ReferenceStability,
    canonical_reference_lineage_digest,
};

pub const EXACT_PRODUCT_SCHEMA_V1: &str = "ketchup.exact-product.v1";
pub const EXACT_BREP_GRAPH_EVALUATOR_V1: &str = "ketchup.exact-brep-graph-evaluator.v1";

impl ExactBRepGraph {
    /// Whether `reference` names a face or edge of the body this graph builds,
    /// regardless of which evaluation produced the stored evidence.
    #[must_use]
    pub fn names_durable_reference(&self, reference: &BodySubshapeRef) -> bool {
        reference.has_valid_lineage()
            && reference.document_id.0 == self.document_id
            && reference.definition_id.0 == self.definition_id
            && reference.producer_feature_id.0 == self.producer_feature_id
            && (self
                .profiles
                .iter()
                .any(|profile| profile.source_feature_id == reference.profile_feature_id.0)
                || reference.profile_feature_id == reference.producer_feature_id)
    }

    /// Whether `reference` names a face or edge of the body this graph builds and
    /// carries the evidence of evaluating exactly this graph.
    #[must_use]
    pub fn names_evaluated_reference(&self, reference: &BodySubshapeRef) -> bool {
        reference.has_valid_lineage()
            && reference.document_id.0 == self.document_id
            && reference.definition_id.0 == self.definition_id
            && reference.producer_feature_id.0 == self.producer_feature_id
            && (self
                .profiles
                .iter()
                .any(|profile| profile.source_feature_id == reference.profile_feature_id.0)
                || (reference.profile_feature_id == reference.producer_feature_id
                    && self.nodes.iter().any(|node| {
                        node.source_feature_id == self.producer_feature_id
                            && matches!(node.operation, ExactBRepOperation::SheetMetal { .. })
                    })))
            && reference.canonical_input_digest == self.canonical_input_digest
            && reference.evaluator == EXACT_BREP_GRAPH_EVALUATOR_V1
            && !reference.exact_input_digest.is_empty()
            && !reference.result_fingerprint.is_empty()
            && !reference.backend.is_empty()
            && !reference.tolerance.is_empty()
            && !reference.corroborating_geometry_fingerprint.is_empty()
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactPlanarFaceAttachmentInput {
    pub role: ExactFaceRole,
    pub local_origin_mm: [f64; 3],
    pub local_unit_normal: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactAxialAttachmentInput {
    pub role: ExactFaceRole,
    pub kind: AxialAttachmentKind,
    pub local_origin_mm: [f64; 3],
    pub local_unit_direction: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedExactPackage {
    pub identity: BodyResultIdentity,
    pub source_sha256: [u8; 32],
    pub source_bytes: Vec<u8>,
    pub source_part_index: Option<u32>,
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
    pub graph: Box<ExactBRepGraph>,
    pub volume_mm3: f64,
    pub area_mm2: f64,
    pub topology_counts: [u32; 5],
    pub bounds_mm: [[f64; 3]; 2],
    pub vertices: Vec<ExactVertex>,
    pub triangles: Vec<ExactTriangle>,
    pub triangle_face_ordinals: Vec<u32>,
    pub topological_references: Vec<TopologicalElementRef>,
    pub face_evidence: Vec<ExactBRepGraphFaceEvidence>,
    pub edge_evidence: Vec<ExactBRepGraphEdgeEvidence>,
    pub references: Vec<BodySubshapeRef>,
    pub planar_face_attachments: Vec<PlanarFaceAttachment>,
    pub axial_attachments: Vec<AxialAttachment>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactBRepGraphFaceEvidence {
    pub semantic_role: String,
    pub source_element_id: String,
    pub face_ordinal: u32,
    pub surface_kind: String,
    pub corroborating_geometry_fingerprint: String,
    pub centroid_mm: [f64; 3],
    pub unit_normal: [f64; 3],
    pub axis_origin_mm: Option<[f64; 3]>,
    pub unit_axis_direction: Option<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactBRepGraphEdgeEvidence {
    pub edge_ordinal: u32,
    pub curve_kind: String,
    pub length_mm: f64,
    pub centroid_mm: [f64; 3],
    pub bounds_mm: [[f64; 3]; 2],
    pub closed: bool,
    pub circle_radius_mm: Option<f64>,
    pub axis_origin_mm: Option<[f64; 3]>,
    pub unit_axis_direction: Option<[f64; 3]>,
    pub adjacent_face_ordinals: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactBRepGraphWorkerEvidence {
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub volume_mm3: f64,
    pub area_mm2: f64,
    pub topology_counts: [u32; 5],
    pub wire_count: Option<u32>,
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
    pub faces: Vec<ExactBRepGraphFaceEvidence>,
    pub edges: Vec<ExactBRepGraphEdgeEvidence>,
}

fn edge_geometry_fingerprint(edge: &ExactBRepGraphEdgeEvidence) -> String {
    let mut geometry = b"ketchup.topological-edge-geometry.v1".to_vec();
    geometry.extend_from_slice(&(edge.curve_kind.len() as u64).to_le_bytes());
    geometry.extend_from_slice(edge.curve_kind.as_bytes());
    geometry.extend_from_slice(&edge.length_mm.to_bits().to_le_bytes());
    for value in edge
        .centroid_mm
        .into_iter()
        .chain(edge.bounds_mm.into_iter().flatten())
    {
        geometry.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    geometry.push(u8::from(edge.closed));
    geometry.push(u8::from(edge.circle_radius_mm.is_some()));
    if let Some(value) = edge.circle_radius_mm {
        geometry.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    for vector in [edge.axis_origin_mm, edge.unit_axis_direction] {
        geometry.push(u8::from(vector.is_some()));
        if let Some(vector) = vector {
            for value in vector {
                geometry.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
    }
    sha256_hex(&geometry)
}

fn publish_graph_topological_references(
    identity: &BodyResultIdentity,
    topology_counts: [u32; 5],
    faces: &[ExactBRepGraphFaceEvidence],
    edges: &[ExactBRepGraphEdgeEvidence],
) -> Result<Vec<TopologicalElementRef>, TopologicalReferenceError> {
    let mut references = publish_generated_topological_references(identity, topology_counts)?;
    let mut face_provenance = BTreeMap::<u32, Vec<(&str, &str)>>::new();
    for face in faces {
        face_provenance
            .entry(face.face_ordinal)
            .or_default()
            .push((&face.semantic_role, &face.source_element_id));
    }

    for (ordinal, reference) in references
        .iter_mut()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .enumerate()
    {
        let Some([face]) = face_provenance.get(&(ordinal as u32)).map(Vec::as_slice) else {
            continue;
        };
        let Some(evidence) = faces.iter().find(|candidate| {
            candidate.face_ordinal == ordinal as u32
                && candidate.semantic_role == face.0
                && candidate.source_element_id == face.1
        }) else {
            continue;
        };
        if face.0.is_empty() || face.1.is_empty() {
            continue;
        }
        reference.source_element_id = face.1.to_owned();
        reference.producer_element_id = face.0.to_owned();
        reference.stability = TopologicalReferenceStability::Guaranteed;
        reference.corroborating_geometry_fingerprint =
            evidence.corroborating_geometry_fingerprint.clone();
        reference.lineage_digest = canonical_topological_lineage_digest(reference);
    }

    for (ordinal, reference) in references
        .iter_mut()
        .filter(|reference| reference.kind == TopologicalElementKind::Edge)
        .enumerate()
    {
        let Some(edge) = edges
            .iter()
            .find(|candidate| candidate.edge_ordinal == ordinal as u32)
        else {
            continue;
        };
        let adjacent_faces = edge
            .adjacent_face_ordinals
            .iter()
            .map(|face_ordinal| {
                let [face] = face_provenance.get(face_ordinal)?.as_slice() else {
                    return None;
                };
                Some(*face)
            })
            .collect::<Option<Vec<_>>>();
        let Some((source_element_id, producer_element_id)) = adjacent_faces
            .as_deref()
            .and_then(topological_edge_provenance_tokens)
        else {
            continue;
        };
        reference.source_element_id = source_element_id;
        reference.producer_element_id = producer_element_id;
        reference.stability = TopologicalReferenceStability::Guaranteed;
        reference.corroborating_geometry_fingerprint = edge_geometry_fingerprint(edge);
        reference.lineage_digest = canonical_topological_lineage_digest(reference);
    }
    Ok(references)
}

fn is_finite_unit_vector(vector: [f64; 3]) -> bool {
    vector.into_iter().all(f64::is_finite)
        && (vector.into_iter().map(|value| value * value).sum::<f64>() - 1.0).abs() <= ROUNDING
}

impl ExactBRepGraphPackage {
    pub fn from_worker_evidence(
        graph: &ExactBRepGraph,
        evidence: ExactBRepGraphWorkerEvidence,
        mesh: &StepImportMesh,
    ) -> Result<Self, ExactProductError> {
        graph
            .validate()
            .map_err(|error| ExactProductError::InvalidWorkerEvidence.because(error))?;
        let vertex_count = mesh.vertices_mm.len();
        let valid_terminal = if graph.terminal_is_planar_offset() {
            evidence.volume_mm3.is_finite()
                && evidence.volume_mm3 == 0.0
                && graph.accepts_terminal_planar_offset_geometry(
                    evidence.bounds_mm,
                    graph.terminal_planar_offset_local_bounds_mm(&mesh.vertices_mm),
                    evidence.area_mm2,
                    evidence.topology_counts,
                    evidence.wire_count,
                )
        } else if graph.terminal_is_surface() {
            evidence.volume_mm3.is_finite()
                && evidence.volume_mm3 == 0.0
                && evidence.area_mm2.is_finite()
                && evidence.area_mm2 > 0.0
                && evidence.topology_counts[..3].iter().all(|count| *count > 0)
                && evidence.topology_counts[4] == 0
                && evidence
                    .bounds_mm
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
                && (0..3).all(|axis| evidence.bounds_mm[0][axis] <= evidence.bounds_mm[1][axis])
                && (0..3)
                    .filter(|axis| evidence.bounds_mm[0][*axis] < evidence.bounds_mm[1][*axis])
                    .count()
                    >= 2
        } else {
            evidence.volume_mm3.is_finite()
                && evidence.volume_mm3 > 0.0
                && evidence.area_mm2.is_finite()
                && evidence.area_mm2 == 0.0
                && !evidence.topology_counts.contains(&0)
                && evidence
                    .bounds_mm
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
                && (0..3).all(|axis| evidence.bounds_mm[0][axis] < evidence.bounds_mm[1][axis])
        };
        if evidence.exact_input_digest.is_empty()
            || evidence.result_fingerprint.is_empty()
            || evidence.backend.is_empty()
            || evidence.tolerance.is_empty()
            || !valid_terminal
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
        if graph.terminal_is_surface() {
            let mesh_area_mm2 = mesh.triangles.iter().fold(0.0, |area, triangle| {
                let [a, b, c] = triangle
                    .vertex_indices
                    .map(|index| mesh.vertices_mm[index as usize]);
                let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let cross = ketchup_geometry::linalg::cross(ab, ac);
                area + 0.5 * cross[0].hypot(cross[1]).hypot(cross[2])
            });
            let area_tolerance_mm2 = evidence.area_mm2.max(mesh_area_mm2).max(1.0) * 0.01;
            if !mesh_area_mm2.is_finite()
                || (mesh_area_mm2 - evidence.area_mm2).abs() > area_tolerance_mm2
            {
                return Err(ExactProductError::InvalidWorkerEvidence);
            }
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
        let mut edge_ordinals = BTreeSet::new();
        for edge in &evidence.edges {
            let finite_bounds = edge
                .bounds_mm
                .iter()
                .flatten()
                .all(|value| value.is_finite())
                && (0..3).all(|axis| edge.bounds_mm[0][axis] <= edge.bounds_mm[1][axis]);
            let circle_fields_match =
                edge.circle_radius_mm.is_some() == (edge.curve_kind == "circle");
            let axis_fields_match = edge.axis_origin_mm.is_some()
                == edge.unit_axis_direction.is_some()
                && edge.axis_origin_mm.is_some()
                    == matches!(edge.curve_kind.as_str(), "line" | "circle");
            if edge.edge_ordinal >= evidence.topology_counts[1]
                || !edge_ordinals.insert(edge.edge_ordinal)
                || edge.curve_kind.is_empty()
                || !edge.length_mm.is_finite()
                || edge.length_mm <= 0.0
                || !edge.centroid_mm.into_iter().all(f64::is_finite)
                || !finite_bounds
                || !circle_fields_match
                || !axis_fields_match
                || edge
                    .circle_radius_mm
                    .is_some_and(|radius| !radius.is_finite() || radius <= 0.0)
                || edge
                    .axis_origin_mm
                    .is_some_and(|origin| !origin.into_iter().all(f64::is_finite))
                || edge
                    .unit_axis_direction
                    .is_some_and(|direction| !is_finite_unit_vector(direction))
                || edge
                    .adjacent_face_ordinals
                    .iter()
                    .any(|ordinal| *ordinal >= evidence.topology_counts[2])
            {
                return Err(ExactProductError::InvalidWorkerEvidence);
            }
        }
        let mut references = Vec::new();
        let mut planar_face_attachments = Vec::new();
        let mut axial_attachments = Vec::new();
        for face in &evidence.faces {
            if face.face_ordinal >= evidence.topology_counts[2]
                || face.corroborating_geometry_fingerprint.is_empty()
                || !face.centroid_mm.into_iter().all(f64::is_finite)
                || (face.surface_kind == "plane" && !is_finite_unit_vector(face.unit_normal))
                || (face.surface_kind == "cylinder"
                    && !face.unit_normal.into_iter().all(f64::is_finite))
                || face.axis_origin_mm.is_some() != face.unit_axis_direction.is_some()
                || face
                    .axis_origin_mm
                    .is_some_and(|origin| !origin.into_iter().all(f64::is_finite))
                || face
                    .unit_axis_direction
                    .is_some_and(|direction| !is_finite_unit_vector(direction))
            {
                return Err(ExactProductError::InvalidWorkerEvidence);
            }
            let expected_type = match face.surface_kind.as_str() {
                "plane" => "planar_face",
                "cylinder" if face.axis_origin_mm.is_some() => "cylindrical_face",
                _ => continue,
            };
            let reference = BodySubshapeRef {
                schema: BODY_SUBSHAPE_REF_SCHEMA_V1.to_owned(),
                document_id: identity.document_id,
                definition_id: identity.definition_id,
                profile_feature_id: identity.profile_feature_id,
                producer_feature_id: identity.producer_feature_id,
                semantic_role: face.semantic_role.clone(),
                source_element_id: face.source_element_id.clone(),
                expected_type: expected_type.to_owned(),
                expected_cardinality: 1,
                stability: ReferenceStability::Guaranteed,
                canonical_input_digest: identity.canonical_input_digest.clone(),
                exact_input_digest: identity.exact_input_digest.clone(),
                result_fingerprint: identity.result_fingerprint.clone(),
                evaluator: identity.evaluator.clone(),
                backend: identity.backend.clone(),
                tolerance: identity.tolerance.clone(),
                lineage_digest: String::new(),
                corroborating_geometry_fingerprint: face.corroborating_geometry_fingerprint.clone(),
            };
            let mut reference = reference;
            reference.lineage_digest = reference_lineage_digest(&reference);
            if !reference.has_valid_lineage()
                || references.iter().any(|existing: &BodySubshapeRef| {
                    existing.semantic_role == reference.semantic_role
                        && existing.source_element_id == reference.source_element_id
                })
            {
                continue;
            }
            if expected_type == "planar_face" {
                let attachment = PlanarFaceAttachment::new(
                    reference.clone(),
                    face.centroid_mm,
                    face.unit_normal,
                )
                .ok_or(ExactProductError::InvalidWorkerEvidence)?;
                planar_face_attachments.push(attachment);
            } else {
                let attachment = AxialAttachment::cylindrical_face(
                    reference.clone(),
                    face.axis_origin_mm.expect("validated axis origin"),
                    face.unit_axis_direction.expect("validated axis direction"),
                )
                .ok_or(ExactProductError::InvalidWorkerEvidence)?;
                axial_attachments.push(attachment);
            }
            references.push(reference);
        }
        let topological_references = publish_graph_topological_references(
            &identity,
            evidence.topology_counts,
            &evidence.faces,
            &evidence.edges,
        )
        .map_err(|error| ExactProductError::InvalidWorkerEvidence.because(error))?;
        // Triangles of a named face carry its role so picks resolve to a
        // durable face reference.
        let face_roles = evidence
            .faces
            .iter()
            .filter_map(|face| {
                references
                    .iter()
                    .find(|reference| {
                        reference.semantic_role == face.semantic_role
                            && reference.source_element_id == face.source_element_id
                    })
                    .and_then(BodySubshapeRef::role)
                    .map(|role| (face.face_ordinal, role))
            })
            .collect::<BTreeMap<_, _>>();
        Ok(Self {
            identity,
            graph: Box::new(graph.clone()),
            volume_mm3: evidence.volume_mm3,
            area_mm2: evidence.area_mm2,
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
                    face_role: face_roles.get(&triangle.face_ordinal).copied(),
                })
                .collect(),
            triangle_face_ordinals: mesh
                .triangles
                .iter()
                .map(|triangle| triangle.face_ordinal)
                .collect(),
            topological_references,
            face_evidence: evidence.faces,
            edge_evidence: evidence.edges,
            references,
            planar_face_attachments,
            axial_attachments,
        })
    }

    fn matches_graph(&self, graph: &ExactBRepGraph) -> bool {
        self.identity.document_id == DocumentId(graph.document_id)
            && self.identity.definition_id == DefinitionId(graph.definition_id)
            && self.identity.producer_feature_id == FeatureId(graph.producer_feature_id)
            && self.identity.canonical_input_digest == graph.canonical_input_digest
            && self.identity.evaluator == EXACT_BREP_GRAPH_EVALUATOR_V1
            && self.graph.as_ref() == graph
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && snapshot
                .exact_brep_graph(
                    self.identity.definition_id,
                    self.identity.producer_feature_id,
                )
                .is_some_and(|graph| self.matches_graph(&graph))
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
        for reference in &mut rebound.references {
            reference.canonical_input_digest = graph.canonical_input_digest.clone();
        }
        rebound.planar_face_attachments = self
            .planar_face_attachments
            .iter()
            .map(|attachment| {
                let reference = rebound.references.iter().find(|reference| {
                    reference.semantic_role == attachment.reference().semantic_role
                        && reference.source_element_id == attachment.reference().source_element_id
                })?;
                PlanarFaceAttachment::new(
                    reference.clone(),
                    attachment.local_origin_mm(),
                    attachment.local_unit_normal(),
                )
            })
            .collect::<Option<Vec<_>>>()?;
        rebound.axial_attachments = self
            .axial_attachments
            .iter()
            .map(|attachment| {
                let reference = rebound.references.iter().find(|reference| {
                    reference.semantic_role == attachment.reference().semantic_role
                        && reference.source_element_id == attachment.reference().source_element_id
                })?;
                AxialAttachment::new(
                    reference.clone(),
                    attachment.kind(),
                    attachment.local_origin_mm(),
                    attachment.local_unit_direction(),
                )
            })
            .collect::<Option<Vec<_>>>()?;
        rebound.graph = Box::new(graph);
        Some(rebound)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExactBodyPackage {
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

#[derive(Clone, Copy)]
pub enum MeshExportSource<'a> {
    Exact(&'a ExactBodyPackage),
    Canonical {
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
        mesh: &'a MeshBodySpec,
    },
}

impl MeshExportSource<'_> {
    #[must_use]
    pub fn definition_id(self) -> DefinitionId {
        match self {
            Self::Exact(package) => package.definition_id(),
            Self::Canonical { definition_id, .. } => definition_id,
        }
    }

    #[must_use]
    pub fn producer_feature_id(self) -> FeatureId {
        match self {
            Self::Exact(package) => package.producer_feature_id(),
            Self::Canonical {
                producer_feature_id,
                ..
            } => producer_feature_id,
        }
    }

    #[must_use]
    pub fn is_current(self, snapshot: &Snapshot) -> bool {
        match self {
            Self::Exact(package) => package.is_current(snapshot),
            Self::Canonical {
                definition_id,
                producer_feature_id,
                mesh,
            } => snapshot.feature(producer_feature_id).is_some_and(|feature| {
                feature.definition_id() == definition_id
                    && matches!(feature.kind(), FeatureKind::MeshBody(current) if current == mesh)
            }),
        }
    }

    #[must_use]
    pub fn vertex_count(self) -> usize {
        match self {
            Self::Exact(package) => package.vertices().len(),
            Self::Canonical { mesh, .. } => mesh.vertices_mm.len(),
        }
    }

    #[must_use]
    pub fn vertex_position_mm(self, index: usize) -> Option<[f64; 3]> {
        match self {
            Self::Exact(package) => package
                .vertices()
                .get(index)
                .map(|vertex| vertex.position_mm),
            Self::Canonical { mesh, .. } => mesh.vertices_mm.get(index).copied(),
        }
    }

    #[must_use]
    pub fn triangle_count(self) -> usize {
        match self {
            Self::Exact(package) => package.triangles().len(),
            Self::Canonical { mesh, .. } => mesh.triangles.len(),
        }
    }

    #[must_use]
    pub fn triangle_indices(self, index: usize) -> Option<[u32; 3]> {
        match self {
            Self::Exact(package) => package
                .triangles()
                .get(index)
                .map(|triangle| triangle.vertex_indices),
            Self::Canonical { mesh, .. } => mesh.triangles.get(index).copied(),
        }
    }

    #[must_use]
    pub fn identity(self) -> String {
        match self {
            Self::Exact(package) => format!("exact:{}", package.result_fingerprint()),
            Self::Canonical { .. } => "canonical-mesh".to_owned(),
        }
    }

    #[must_use]
    pub const fn is_canonical(self) -> bool {
        matches!(self, Self::Canonical { .. })
    }
}

#[derive(Clone, Copy)]
pub struct MeshExportBody<'a> {
    pub source: MeshExportSource<'a>,
    pub transform: Transform,
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
const IMPORTED_MESH_BOUNDS_TOLERANCE_MM: f64 = APPROXIMATION;

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
            .map_err(|error| ExactProductError::InvalidWorkerEvidence.because(error))?
            .unwrap_or_default();
        Ok(Self {
            identity,
            source_sha256,
            source_bytes,
            source_part_index: spec.source_part_index,
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
                            && spec.source_part_index == self.source_part_index
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
            Self::Graph(package) => package.identity.definition_id,
            Self::Imported(package) => package.identity.definition_id,
        }
    }

    #[must_use]
    pub fn producer_feature_id(&self) -> FeatureId {
        match self {
            Self::Graph(package) => package.identity.producer_feature_id,
            Self::Imported(package) => package.identity.producer_feature_id,
        }
    }

    #[must_use]
    pub fn result_key(&self) -> ExactResultKey {
        match self {
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
            Self::Graph(package) => package.is_current(snapshot),
            Self::Imported(package) => package.is_current(snapshot),
        }
    }

    #[must_use]
    pub fn rebound_to(&self, snapshot: &Snapshot) -> Option<Self> {
        match self {
            Self::Graph(package) => package.rebound_to(snapshot).map(Self::Graph),
            Self::Imported(package) => package.rebound_to(snapshot).map(Self::Imported),
        }
    }

    #[must_use]
    pub fn bounds_mm(&self) -> [[f64; 3]; 2] {
        match self {
            Self::Graph(package) => package.bounds_mm,
            Self::Imported(package) => package.bounds_mm,
        }
    }

    #[must_use]
    pub fn vertices(&self) -> &[ExactVertex] {
        match self {
            Self::Graph(package) => &package.vertices,
            Self::Imported(package) => &package.vertices,
        }
    }

    #[must_use]
    pub fn triangles(&self) -> &[ExactTriangle] {
        match self {
            Self::Graph(package) => &package.triangles,
            Self::Imported(package) => &package.triangles,
        }
    }

    #[must_use]
    pub fn references(&self) -> &[BodySubshapeRef] {
        match self {
            Self::Graph(package) => &package.references,
            Self::Imported(_) => &[],
        }
    }

    #[must_use]
    pub fn topological_references(&self) -> &[TopologicalElementRef] {
        match self {
            Self::Graph(package) => &package.topological_references,
            Self::Imported(package) => &package.topological_references,
        }
    }

    #[must_use]
    pub fn edge_evidence(&self) -> &[ExactBRepGraphEdgeEvidence] {
        match self {
            Self::Graph(package) => &package.edge_evidence,
            Self::Imported(_) => &[],
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
        }
    }

    fn tolerance(&self) -> &str {
        match self {
            Self::Graph(package) => &package.identity.tolerance,
            Self::Imported(package) => &package.identity.tolerance,
        }
    }

    fn source_digest(&self) -> &str {
        match self {
            Self::Graph(package) => &package.identity.source_digest,
            Self::Imported(package) => &package.identity.source_digest,
        }
    }

    fn producer_identity(&self) -> String {
        format!("producer_feature_id={}", self.producer_feature_id().0)
    }

    fn result_fingerprint(&self) -> &str {
        match self {
            Self::Graph(package) => &package.identity.result_fingerprint,
            Self::Imported(package) => &package.identity.result_fingerprint,
        }
    }
}

fn mesh_export_from_view(
    view: &(impl ExactBodyView + ?Sized),
    transform: Transform,
) -> ExactMeshExport {
    let mirrored = transform.affine().determinant() < 0.0;
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
        let [x, y, z] = transform.transform_point(view.vertex_position_mm(index));
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
        if mirrored {
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

const MAX_STL_EXPORT_BYTES: usize = 256 * 1024 * 1024;

pub fn exact_model_stl_export(
    snapshot: &Snapshot,
    bodies: &[(&ExactBodyPackage, Transform)],
) -> Result<ExactStlExport, ExactProductError> {
    let bodies = bodies
        .iter()
        .map(|(package, transform)| MeshExportBody {
            source: MeshExportSource::Exact(package),
            transform: *transform,
        })
        .collect::<Vec<_>>();
    model_stl_export(snapshot, &bodies)
}

pub fn model_stl_export(
    snapshot: &Snapshot,
    bodies: &[MeshExportBody<'_>],
) -> Result<ExactStlExport, ExactProductError> {
    if bodies.is_empty() {
        return Err(ExactProductError::EmptyModelExport);
    }
    if bodies.len() > limits::EXPORT_INSTANCES {
        return Err(ExactProductError::ExportResourceLimit);
    }
    if bodies.iter().any(|body| !body.source.is_current(snapshot)) {
        return Err(ExactProductError::StaleResult);
    }
    let total_triangles = bodies.iter().try_fold(0_usize, |count, body| {
        count
            .checked_add(body.source.triangle_count())
            .filter(|total| *total <= limits::EXPORT_TRIANGLES)
            .ok_or(ExactProductError::ExportResourceLimit)
    })?;

    let mut mesh_stl = String::from("solid ketchup_current_model\n");
    let mut facet_count = 0_usize;
    for body in bodies {
        let source = body.source;
        let matrix = body.transform.matrix();
        let linear = body.transform.affine().linear;
        if matrix.iter().any(|value| !value.is_finite()) || linear.is_singular() {
            return Err(ExactProductError::InvalidMeshExport);
        }
        let mirrored = linear.determinant() < 0.0;
        for triangle_index in 0..source.triangle_count() {
            let mut indices = source
                .triangle_indices(triangle_index)
                .ok_or(ExactProductError::InvalidMeshExport)?;
            if mirrored {
                indices.swap(1, 2);
            }
            let points = indices.map(|index| {
                source
                    .vertex_position_mm(index as usize)
                    .map(|point| body.transform.transform_point(point))
            });
            let [Some(a), Some(b), Some(c)] = points else {
                return Err(ExactProductError::InvalidMeshExport);
            };
            let ab = ketchup_geometry::linalg::sub(b, a);
            let ac = ketchup_geometry::linalg::sub(c, a);
            let cross = ketchup_geometry::linalg::cross(ab, ac);
            let length = ketchup_geometry::linalg::length(cross);
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
            if mesh_stl.len() > MAX_STL_EXPORT_BYTES {
                return Err(ExactProductError::ExportResourceLimit);
            }
        }
    }
    mesh_stl.push_str("endsolid ketchup_current_model\n");

    let source_identities = bodies
        .iter()
        .map(|body| body.source.identity())
        .collect::<Vec<_>>()
        .join(",");
    let canonical_mesh_count = bodies
        .iter()
        .filter(|body| body.source.is_canonical())
        .count();
    let loss_report = format!(
        "authority=validated exact tessellations and canonical mesh bodies\nformat=ASCII STL\nconversion=current-visible-model-to-world-space-mesh\ncolor_loss=STL does not preserve occurrence colors\neditability_loss=canonical features, rules, dimensions, hierarchy, and Undo history are not preserved\ntopology_loss=exact topology, analytic surfaces, assembly identity, and durable face identity are not preserved\ntolerance_loss=exact geometry uses its accepted tessellation; canonical mesh vertices are preserved\nsource_digest={}\noccurrence_count={}\ncanonical_mesh_occurrence_count={canonical_mesh_count}\nfacet_count={facet_count}\nsource_identities={source_identities}\nresource_triangle_limit={}\n",
        snapshot.canonical_digest(),
        bodies.len(),
        limits::EXPORT_TRIANGLES,
    );
    debug_assert_eq!(facet_count, total_triangles);
    Ok(ExactStlExport {
        mesh_stl,
        loss_report,
    })
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
    contents_stamp: u64,
}

impl Default for ExactResultRegistry {
    fn default() -> Self {
        Self {
            packages: BTreeMap::new(),
            contents_stamp: next_contents_stamp(),
        }
    }
}

fn next_contents_stamp() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub struct ExactSnapshotPreparation<'a> {
    snapshot: &'a Snapshot,
    dependencies: FeatureDependencyGraph,
}

impl<'a> ExactSnapshotPreparation<'a> {
    pub fn new(snapshot: &'a Snapshot) -> Result<Self, ExactProductError> {
        let dependencies = snapshot
            .feature_dependency_graph()
            .map_err(|error| ExactProductError::UnsupportedDefinition.because(error))?;
        Ok(Self {
            snapshot,
            dependencies,
        })
    }

    pub fn terminal_features(
        &self,
        definition_id: DefinitionId,
    ) -> Result<BTreeMap<BodyId, FeatureId>, ExactProductError> {
        exact_body_terminal_features_with_graph(self.snapshot, definition_id, &self.dependencies)
    }

    pub fn graph(
        &self,
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
    ) -> Result<ExactBRepGraph, ExactBRepGraphError> {
        ExactBRepGraph::from_snapshot_with_dependencies(
            self.snapshot,
            definition_id,
            producer_feature_id,
            &self.dependencies,
        )
    }
}

pub fn exact_body_terminal_features(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Result<BTreeMap<BodyId, FeatureId>, ExactProductError> {
    let graph = snapshot
        .feature_dependency_graph()
        .map_err(|error| ExactProductError::UnsupportedDefinition.because(error))?;
    exact_body_terminal_features_with_graph(snapshot, definition_id, &graph)
}

/// Compiles the exact B-Rep graph that produces `producer_feature_id`.
pub fn producer_exact_graph(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    producer_feature_id: FeatureId,
) -> Result<ExactBRepGraph, ExactProductError> {
    if snapshot.feature_is_suppressed(producer_feature_id) {
        return Err(ExactProductError::UnsupportedDefinition);
    }
    ExactBRepGraph::from_snapshot(snapshot, definition_id, producer_feature_id)
        .map_err(|error| ExactProductError::UnsupportedDefinition.because(error))
}

/// Compiles the exact B-Rep graph of the terminal producer of `body_id`.
pub fn body_exact_graph(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    body_id: BodyId,
) -> Result<ExactBRepGraph, ExactProductError> {
    let producer_feature_id = exact_body_terminal_features(snapshot, definition_id)?
        .get(&body_id)
        .copied()
        .ok_or(ExactProductError::BodyOutputNotFound {
            definition_id,
            body_id,
        })?;
    producer_exact_graph(snapshot, definition_id, producer_feature_id)
}

/// Compiles the exact B-Rep graph of every terminal body of a definition.
pub fn terminal_body_exact_graphs(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Result<BTreeMap<BodyId, ExactBRepGraph>, ExactProductError> {
    exact_body_terminal_features(snapshot, definition_id)?
        .into_iter()
        .map(|(body_id, producer_feature_id)| {
            producer_exact_graph(snapshot, definition_id, producer_feature_id)
                .map(|graph| (body_id, graph))
        })
        .collect()
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
            .map_err(|error| ExactProductError::UnsupportedDefinition.because(error))?;
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
    pub fn planar_face_attachment<'a>(
        &'a self,
        snapshot: &Snapshot,
        reference: &BodySubshapeRef,
    ) -> Option<&'a PlanarFaceAttachment> {
        let key = ExactResultKey {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            definition_id: reference.definition_id,
            producer_feature_id: reference.producer_feature_id,
            canonical_input_digest: reference.canonical_input_digest.clone(),
            exact_input_digest: reference.exact_input_digest.clone(),
            evaluator: reference.evaluator.clone(),
            backend: reference.backend.clone(),
            tolerance: reference.tolerance.clone(),
            schema: EXACT_PRODUCT_SCHEMA_V1.to_owned(),
            result_fingerprint: reference.result_fingerprint.clone(),
        };
        match self.packages.get(&key)?.as_ref() {
            ExactBodyPackage::Graph(package) => package
                .planar_face_attachments
                .iter()
                .find(|attachment| attachment.reference() == reference),
            ExactBodyPackage::Imported(_) => None,
        }
    }

    #[must_use]
    pub fn axial_attachment<'a>(
        &'a self,
        snapshot: &Snapshot,
        reference: &BodySubshapeRef,
    ) -> Option<&'a AxialAttachment> {
        let key = ExactResultKey {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            definition_id: reference.definition_id,
            producer_feature_id: reference.producer_feature_id,
            canonical_input_digest: reference.canonical_input_digest.clone(),
            exact_input_digest: reference.exact_input_digest.clone(),
            evaluator: reference.evaluator.clone(),
            backend: reference.backend.clone(),
            tolerance: reference.tolerance.clone(),
            schema: EXACT_PRODUCT_SCHEMA_V1.to_owned(),
            result_fingerprint: reference.result_fingerprint.clone(),
        };
        match self.packages.get(&key)?.as_ref() {
            ExactBodyPackage::Graph(package) => package
                .axial_attachments
                .iter()
                .find(|attachment| attachment.reference() == reference),
            ExactBodyPackage::Imported(_) => None,
        }
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

    #[must_use]
    pub fn len(&self) -> usize {
        self.packages.len()
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
        self.contents_stamp = next_contents_stamp();
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

impl ExactCircleProfile {}

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

fn exact_segment_tangents(segment: &ExactProfileSegment) -> Option<([f64; 2], [f64; 2])> {
    let normalized = |vector: [f64; 2]| {
        let length = vector[0].hypot(vector[1]);
        (length.is_finite() && length >= limits::MIN_LENGTH_MM)
            .then_some([vector[0] / length, vector[1] / length])
    };
    match segment {
        ExactProfileSegment::Line {
            start_bits,
            end_bits,
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let tangent = normalized([end[0] - start[0], end[1] - start[1]])?;
            Some((tangent, tangent))
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
            let tangent = |point: [f64; 2]| {
                let radial = normalized([point[0] - center[0], point[1] - center[1]])?;
                Some(if *clockwise {
                    [radial[1], -radial[0]]
                } else {
                    [-radial[1], radial[0]]
                })
            };
            Some((tangent(start)?, tangent(end)?))
        }
    }
}

impl ExactMixedProfile {
    #[must_use]
    pub fn has_only_line_segments(&self) -> bool {
        self.segments
            .iter()
            .all(|segment| matches!(segment, ExactProfileSegment::Line { .. }))
    }

    #[must_use]
    pub fn max_planar_offset_displacement_mm(&self, distance_mm: f64) -> Option<f64> {
        let distance = distance_mm.abs();
        let mut max_factor = 1.0_f64;
        let mut has_arc = false;
        for segment in &self.segments {
            if let ExactProfileSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } = segment
            {
                has_arc = true;
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                if distance_mm < 0.0 && 2.0 * distance >= radius {
                    return None;
                }
            }
        }
        for index in 0..self.segments.len() {
            let previous = &self.segments[(index + self.segments.len() - 1) % self.segments.len()];
            let current = &self.segments[index];
            let (_, previous_end) = exact_segment_tangents(previous)?;
            let (current_start, _) = exact_segment_tangents(current)?;
            let dot = (-previous_end[0] * current_start[0] - previous_end[1] * current_start[1])
                .clamp(-1.0, 1.0);
            let half_angle_sine = (0.5 * dot.acos()).sin();
            if !half_angle_sine.is_finite() || half_angle_sine <= ROUNDING {
                return None;
            }
            max_factor = max_factor.max(half_angle_sine.recip());
        }
        let curvature_allowance = if has_arc { distance } else { 0.0 };
        let displacement = distance * max_factor + curvature_allowance;
        displacement.is_finite().then_some(displacement)
    }

    #[must_use]
    pub fn is_strict_convex_line_arc_profile(&self, tolerance: TolerancePolicy) -> bool {
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
        let rounding = scale * scale * ROUNDING;
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
                    if sweep.abs() > std::f64::consts::PI + ROUNDING {
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
                    tolerance.linear_mm(),
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
            if cross.abs() <= rounding {
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
}

pub(crate) trait PlanarOffsetEvidence {
    fn bounds_bits(&self) -> [u64; 4];
    fn area_bits(&self) -> u64;
    fn max_displacement_mm(&self, distance_mm: f64) -> Option<f64>;
}

impl PlanarOffsetEvidence for ExactMixedProfile {
    fn bounds_bits(&self) -> [u64; 4] {
        self.bounds_bits
    }

    fn area_bits(&self) -> u64 {
        self.area_bits
    }

    fn max_displacement_mm(&self, distance_mm: f64) -> Option<f64> {
        self.max_planar_offset_displacement_mm(distance_mm)
    }
}

pub(crate) fn accepts_planar_offset_geometry(
    profile: &impl PlanarOffsetEvidence,
    distance_mm: f64,
    bounds_mm: [[f64; 3]; 2],
    area_mm2: f64,
    topology_counts: [u32; 5],
) -> bool {
    const TOLERANCE: f64 = APPROXIMATION;

    let [source_min_x, source_min_y, source_max_x, source_max_y] =
        profile.bounds_bits().map(f64::from_bits);
    let source_area_mm2 = f64::from_bits(profile.area_bits());
    let minimum_displacement = distance_mm.abs();
    let Some(maximum_displacement) = profile.max_displacement_mm(distance_mm) else {
        return false;
    };
    let [min, max] = bounds_mm;
    let bounds_area = (max[0] - min[0]) * (max[1] - min[1]);
    let within_displacement = |value: f64| {
        value >= minimum_displacement - TOLERANCE && value <= maximum_displacement + TOLERANCE
    };
    let sign_relation = if distance_mm > 0.0 {
        [
            source_min_x - min[0],
            source_min_y - min[1],
            max[0] - source_max_x,
            max[1] - source_max_y,
        ]
        .into_iter()
        .all(within_displacement)
            && area_mm2 > source_area_mm2 + TOLERANCE
    } else {
        [
            min[0] - source_min_x,
            min[1] - source_min_y,
            source_max_x - max[0],
            source_max_y - max[1],
        ]
        .into_iter()
        .all(within_displacement)
            && area_mm2 < source_area_mm2 - TOLERANCE
    };

    min.into_iter()
        .chain(max)
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
        && min[0] < max[0]
        && min[1] < max[1]
        && min[2].abs() <= TOLERANCE
        && max[2].abs() <= TOLERANCE
        && area_mm2.is_finite()
        && area_mm2 > 0.0
        && area_mm2 <= bounds_area + TOLERANCE
        && topology_counts[0] != 0
        && topology_counts[0] == topology_counts[1]
        && topology_counts[2..] == [1, 0, 0]
        && sign_relation
}

pub(crate) fn accepts_planar_circle_offset_geometry(
    circle: ExactCircleProfile,
    distance_mm: f64,
    bounds_mm: [[f64; 3]; 2],
    area_mm2: f64,
    topology_counts: [u32; 5],
) -> bool {
    let center_x = f64::from_bits(circle.center_x_bits);
    let center_y = f64::from_bits(circle.center_y_bits);
    let radius = f64::from_bits(circle.radius_bits) + distance_mm;
    let expected_bounds = [
        [center_x - radius, center_y - radius, 0.0],
        [center_x + radius, center_y + radius, 0.0],
    ];
    let expected_area = std::f64::consts::PI * radius * radius;
    let area_tolerance = APPROXIMATION * expected_area.max(1.0);

    !circle.clockwise
        && [center_x, center_y, radius, distance_mm, area_mm2]
            .into_iter()
            .all(f64::is_finite)
        && (limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&radius)
        && bounds_mm
            .into_iter()
            .flatten()
            .zip(expected_bounds.into_iter().flatten())
            .all(|(actual, expected)| {
                actual.is_finite()
                    && actual.abs() <= MAX_COORDINATE_MM
                    && (actual - expected).abs() <= APPROXIMATION
            })
        && (area_mm2 - expected_area).abs() <= area_tolerance
        && topology_counts == [1, 1, 1, 0, 0]
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactPlanarOffsetProfile {
    pub segments: Vec<ExactBRepPlanarSegment>,
    pub bounds_bits: [u64; 4],
    pub area_bits: u64,
}

impl ExactPlanarOffsetProfile {
    #[must_use]
    pub fn max_planar_offset_displacement_mm(&self, distance_mm: f64) -> Option<f64> {
        let distance = distance_mm.abs();
        let mut max_factor = 1.0_f64;
        let mut has_arc = false;
        for segment in &self.segments {
            if let ExactBRepPlanarSegment::CircularArc {
                start_bits,
                center_bits,
                ..
            } = segment
            {
                has_arc = true;
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                if distance_mm < 0.0 && 2.0 * distance >= radius {
                    return None;
                }
            }
        }
        for index in 0..self.segments.len() {
            let previous = &self.segments[(index + self.segments.len() - 1) % self.segments.len()];
            let current = &self.segments[index];
            let (_, previous_end) = planar_offset_segment_tangents(previous)?;
            let (current_start, _) = planar_offset_segment_tangents(current)?;
            let dot = (-previous_end[0] * current_start[0] - previous_end[1] * current_start[1])
                .clamp(-1.0, 1.0);
            let half_angle_sine = (0.5 * dot.acos()).sin();
            if !half_angle_sine.is_finite() || half_angle_sine <= ROUNDING {
                return None;
            }
            max_factor = max_factor.max(half_angle_sine.recip());
        }
        let displacement = distance * max_factor + if has_arc { distance } else { 0.0 };
        displacement.is_finite().then_some(displacement)
    }
}

impl PlanarOffsetEvidence for ExactPlanarOffsetProfile {
    fn bounds_bits(&self) -> [u64; 4] {
        self.bounds_bits
    }

    fn area_bits(&self) -> u64 {
        self.area_bits
    }

    fn max_displacement_mm(&self, distance_mm: f64) -> Option<f64> {
        self.max_planar_offset_displacement_mm(distance_mm)
    }
}

fn planar_offset_segment_endpoints(segment: &ExactBRepPlanarSegment) -> ([u64; 2], [u64; 2]) {
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        }
        | ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            ..
        }
        | ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            end_bits,
            ..
        } => (*start_bits, *end_bits),
    }
}

fn planar_offset_segment_tangents(
    segment: &ExactBRepPlanarSegment,
) -> Option<([f64; 2], [f64; 2])> {
    let normalized = |vectors: [[f64; 2]; 3]| {
        vectors.into_iter().find_map(|vector| {
            let length = vector[0].hypot(vector[1]);
            (length.is_finite() && length >= limits::MIN_LENGTH_MM)
                .then_some([vector[0] / length, vector[1] / length])
        })
    };
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let tangent = normalized([[end[0] - start[0], end[1] - start[1]]; 3])?;
            Some((tangent, tangent))
        }
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let center = center_bits.map(f64::from_bits);
            let tangent = |point: [f64; 2]| {
                let radial = normalized([[point[0] - center[0], point[1] - center[1]]; 3])?;
                Some(if *clockwise {
                    [radial[1], -radial[0]]
                } else {
                    [-radial[1], radial[0]]
                })
            };
            Some((tangent(start)?, tangent(end)?))
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => {
            let start = start_bits.map(f64::from_bits);
            let control_1 = control_1_bits.map(f64::from_bits);
            let control_2 = control_2_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            Some((
                normalized([
                    [control_1[0] - start[0], control_1[1] - start[1]],
                    [control_2[0] - start[0], control_2[1] - start[1]],
                    [end[0] - start[0], end[1] - start[1]],
                ])?,
                normalized([
                    [end[0] - control_2[0], end[1] - control_2[1]],
                    [end[0] - control_1[0], end[1] - control_1[1]],
                    [end[0] - start[0], end[1] - start[1]],
                ])?,
            ))
        }
    }
}

fn planar_offset_segment_signed_area(
    segment: &ExactBRepPlanarSegment,
    origin: [f64; 2],
) -> Option<f64> {
    let rebase = |point: [f64; 2]| [point[0] - origin[0], point[1] - origin[1]];
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } => {
            let start = rebase(start_bits.map(f64::from_bits));
            let end = rebase(end_bits.map(f64::from_bits));
            Some(0.5 * (start[0] * end[1] - end[0] * start[1]))
        }
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } => {
            let start = rebase(start_bits.map(f64::from_bits));
            let end = rebase(end_bits.map(f64::from_bits));
            let center = rebase(center_bits.map(f64::from_bits));
            let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
            let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
            let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
            let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)?;
            Some(
                0.5 * (radius * radius * sweep + center[0] * (end[1] - start[1])
                    - center[1] * (end[0] - start[0])),
            )
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => {
            let cubic = [
                rebase(start_bits.map(f64::from_bits)),
                rebase(control_1_bits.map(f64::from_bits)),
                rebase(control_2_bits.map(f64::from_bits)),
                rebase(end_bits.map(f64::from_bits)),
            ];
            let coefficients = |axis: usize| {
                [
                    cubic[0][axis],
                    3.0 * (cubic[1][axis] - cubic[0][axis]),
                    3.0 * (cubic[0][axis] - 2.0 * cubic[1][axis] + cubic[2][axis]),
                    -cubic[0][axis] + 3.0 * cubic[1][axis] - 3.0 * cubic[2][axis] + cubic[3][axis],
                ]
            };
            let x = coefficients(0);
            let y = coefficients(1);
            let mut integral = 0.0;
            for left_degree in 0..=3 {
                for right_degree in 1..=3 {
                    let denominator = (left_degree + right_degree) as f64;
                    integral += (x[left_degree] * y[right_degree] * right_degree as f64
                        - y[left_degree] * x[right_degree] * right_degree as f64)
                        / denominator;
                }
            }
            Some(0.5 * integral)
        }
    }
}

pub(crate) fn exact_planar_offset_profile_from_segments(
    segments: Vec<ExactBRepPlanarSegment>,
) -> Option<ExactPlanarOffsetProfile> {
    if !(2..=64).contains(&segments.len()) {
        return None;
    }
    let endpoints = segments
        .iter()
        .map(planar_offset_segment_endpoints)
        .collect::<Vec<_>>();
    if endpoints.windows(2).any(|pair| pair[0].1 != pair[1].0)
        || endpoints.last()?.1 != endpoints[0].0
    {
        return None;
    }
    let valid_point = |point: [f64; 2]| {
        point
            .into_iter()
            .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
    };
    let mut points = Vec::new();
    let mut signed_area = 0.0;
    for segment in &segments {
        match segment {
            ExactBRepPlanarSegment::Line {
                start_bits,
                end_bits,
            } => {
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let length = (end[0] - start[0]).hypot(end[1] - start[1]);
                if !valid_point(start)
                    || !valid_point(end)
                    || !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&length)
                {
                    return None;
                }
                points.extend([start, end]);
            }
            ExactBRepPlanarSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => {
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                if !valid_point(start) || !valid_point(end) || !valid_point(center) {
                    return None;
                }
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
                let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
                let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
                let sweep = directed_arc_sweep(start_angle, end_angle, *clockwise)?;
                if !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&radius)
                    || (radius - end_radius).abs() > ROUNDING * radius.max(end_radius).max(1.0)
                {
                    return None;
                }
                points.extend([start, end]);
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
            }
            ExactBRepPlanarSegment::CubicBezier {
                start_bits,
                control_1_bits,
                control_2_bits,
                end_bits,
            } => {
                let cubic = [
                    start_bits.map(f64::from_bits),
                    control_1_bits.map(f64::from_bits),
                    control_2_bits.map(f64::from_bits),
                    end_bits.map(f64::from_bits),
                ];
                if cubic.into_iter().any(|point| !valid_point(point)) {
                    return None;
                }
                let control_polygon_length = (cubic[1][0] - cubic[0][0])
                    .hypot(cubic[1][1] - cubic[0][1])
                    + (cubic[2][0] - cubic[1][0]).hypot(cubic[2][1] - cubic[1][1])
                    + (cubic[3][0] - cubic[2][0]).hypot(cubic[3][1] - cubic[2][1]);
                if !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&control_polygon_length) {
                    return None;
                }
                points.extend([cubic[0], cubic[3]]);
                let curve = CubicBezier::new(cubic);
                let evaluate = |t: f64| curve.eval(t);
                for axis in 0..2 {
                    // The roots of the derivative c1 + 2·c2·t + 3·c3·t².
                    let [_, c, half_b, third_a] = curve.power_coefficients(axis);
                    let (a, b) = (3.0 * third_a, 2.0 * half_b);
                    if a.abs() <= f64::EPSILON {
                        if b.abs() > f64::EPSILON {
                            let t = -c / b;
                            if 0.0 < t && t < 1.0 {
                                points.push(evaluate(t));
                            }
                        }
                    } else {
                        let discriminant = b * b - 4.0 * a * c;
                        if discriminant >= 0.0 {
                            for t in [
                                (-b - discriminant.sqrt()) / (2.0 * a),
                                (-b + discriminant.sqrt()) / (2.0 * a),
                            ] {
                                if 0.0 < t && t < 1.0 {
                                    points.push(evaluate(t));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let origin = endpoints[0].0.map(f64::from_bits);
    let mut compensation = 0.0;
    for segment in &segments {
        let adjusted = planar_offset_segment_signed_area(segment, origin)? - compensation;
        let next = signed_area + adjusted;
        compensation = (next - signed_area) - adjusted;
        signed_area = next;
    }
    let area = signed_area.abs();
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
    if !area.is_finite()
        || area <= ROUNDING
        || bounds.into_iter().any(|value| !value.is_finite())
        || bounds[0] >= bounds[2]
        || bounds[1] >= bounds[3]
    {
        return None;
    }
    Some(ExactPlanarOffsetProfile {
        segments,
        bounds_bits: bounds.map(f64::to_bits),
        area_bits: area.to_bits(),
    })
}

fn exact_planar_offset_profile_from_solved(
    profile: &SolvedSketchRegionProfile,
) -> Option<ExactPlanarOffsetProfile> {
    let segments = match profile {
        SolvedSketchRegionProfile::Polyline(points) => points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
            .map(|(start_mm, end_mm)| ExactBRepPlanarSegment::Line {
                start_bits: start_mm.map(f64::to_bits),
                end_bits: end_mm.map(f64::to_bits),
            })
            .collect(),
        SolvedSketchRegionProfile::Boundary(edges) => edges
            .iter()
            .map(|edge| match edge {
                SolvedSketchRegionEdge::Line { start_mm, end_mm } => ExactBRepPlanarSegment::Line {
                    start_bits: start_mm.map(f64::to_bits),
                    end_bits: end_mm.map(f64::to_bits),
                },
                SolvedSketchRegionEdge::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => ExactBRepPlanarSegment::CircularArc {
                    start_bits: start_mm.map(f64::to_bits),
                    end_bits: end_mm.map(f64::to_bits),
                    center_bits: center_mm.map(f64::to_bits),
                    clockwise: *clockwise,
                },
                SolvedSketchRegionEdge::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => ExactBRepPlanarSegment::CubicBezier {
                    start_bits: start_mm.map(f64::to_bits),
                    control_1_bits: control_1_mm.map(f64::to_bits),
                    control_2_bits: control_2_mm.map(f64::to_bits),
                    end_bits: end_mm.map(f64::to_bits),
                },
            })
            .collect(),
        SolvedSketchRegionProfile::Circle { .. } => return None,
    };
    exact_planar_offset_profile_from_segments(segments)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactPlanarOffsetRegion {
    pub outer: ExactBRepPlanarLoop,
    pub holes: Vec<ExactBRepPlanarLoop>,
}

impl ExactPlanarOffsetRegion {
    fn from_solved(region: &SolvedSketchRegion) -> Option<Self> {
        if region.holes.is_empty() || region.holes.len() > limits::REGION_HOLES {
            return None;
        }
        let outer = exact_planar_offset_loop_from_solved(&region.outer)?;
        let holes = region
            .holes
            .iter()
            .map(exact_planar_offset_loop_from_solved)
            .collect::<Option<Vec<_>>>()?;
        let segment_count = std::iter::once(&outer)
            .chain(&holes)
            .map(planar_offset_loop_segment_count)
            .sum::<usize>();
        (segment_count <= limits::REGION_SEGMENTS).then_some(Self { outer, holes })
    }

    pub(crate) fn has_valid_encoding(&self, distance_mm: f64) -> bool {
        !self.holes.is_empty()
            && self.holes.len() <= limits::REGION_HOLES
            && std::iter::once(&self.outer)
                .chain(&self.holes)
                .map(planar_offset_loop_segment_count)
                .sum::<usize>()
                <= limits::REGION_SEGMENTS
            && planar_offset_loop_is_valid(&self.outer, distance_mm)
            && self
                .holes
                .iter()
                .all(|hole| planar_offset_loop_is_valid(hole, -distance_mm))
            && self.has_valid_inter_loop_clearance(distance_mm)
    }

    fn has_valid_inter_loop_clearance(&self, distance_mm: f64) -> bool {
        let loops = std::iter::once(&self.outer)
            .chain(&self.holes)
            .collect::<Vec<_>>();
        let Some(curves) = loops
            .iter()
            .map(|planar_loop| planar_offset_loop_clearance_curves(planar_loop))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        if curves.iter().map(Vec::len).sum::<usize>() > MAX_PLANAR_OFFSET_CLEARANCE_CURVES {
            return false;
        }
        let displacements = if distance_mm < 0.0 {
            let Some(displacements) = loops
                .iter()
                .enumerate()
                .map(|(index, planar_loop)| {
                    planar_offset_loop_displacement_mm(
                        planar_loop,
                        if index == 0 {
                            distance_mm
                        } else {
                            -distance_mm
                        },
                    )
                })
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            displacements
        } else {
            vec![0.0; loops.len()]
        };
        let mut comparison_count = 0_usize;
        for right in 1..loops.len() {
            let right_point = planar_offset_clearance_curve_start(curves[right][0]);
            if !point_in_planar_offset_clearance_curves(right_point, &curves[0]) {
                return false;
            }
            for left in 0..right {
                let Some(pair_count) = curves[left].len().checked_mul(curves[right].len()) else {
                    return false;
                };
                let Some(next_count) = comparison_count.checked_add(pair_count) else {
                    return false;
                };
                if next_count > MAX_PLANAR_OFFSET_CLEARANCE_COMPARISONS {
                    return false;
                }
                comparison_count = next_count;
                let Some(clearance) =
                    planar_offset_clearance_curves_mm(&curves[left], &curves[right])
                else {
                    return false;
                };
                if clearance <= PLANAR_OFFSET_CLEARANCE_TOLERANCE_MM
                    || (distance_mm < 0.0
                        && clearance <= displacements[left] + displacements[right])
                {
                    return false;
                }
                if left > 0 {
                    let left_point = planar_offset_clearance_curve_start(curves[left][0]);
                    if point_in_planar_offset_clearance_curves(right_point, &curves[left])
                        || point_in_planar_offset_clearance_curves(left_point, &curves[right])
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn source_bounds_bits(&self) -> Option<[u64; 4]> {
        planar_offset_loop_bounds_bits(&self.outer)
    }

    fn max_outer_displacement_mm(&self, distance_mm: f64) -> Option<f64> {
        match &self.outer {
            ExactBRepPlanarLoop::Boundary { segments } => {
                exact_planar_offset_profile_from_segments(segments.clone())?
                    .max_planar_offset_displacement_mm(distance_mm)
            }
            ExactBRepPlanarLoop::Circle { .. } => Some(distance_mm.abs()),
        }
    }

    fn source_area_bits(&self) -> Option<u64> {
        let outer_area = planar_offset_loop_area_mm2(&self.outer)?;
        let hole_area = self.holes.iter().try_fold(0.0, |area, hole| {
            Some(area + planar_offset_loop_area_mm2(hole)?)
        })?;
        let area = outer_area - hole_area;
        (area.is_finite() && area > 0.0).then(|| area.to_bits())
    }
}

impl PlanarOffsetEvidence for ExactPlanarOffsetRegion {
    fn bounds_bits(&self) -> [u64; 4] {
        self.source_bounds_bits().unwrap_or([f64::NAN.to_bits(); 4])
    }

    fn area_bits(&self) -> u64 {
        self.source_area_bits().unwrap_or(f64::NAN.to_bits())
    }

    fn max_displacement_mm(&self, distance_mm: f64) -> Option<f64> {
        self.max_outer_displacement_mm(distance_mm)
    }
}

fn exact_planar_offset_loop_from_solved(
    profile: &SolvedSketchRegionProfile,
) -> Option<ExactBRepPlanarLoop> {
    match profile {
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => Some(ExactBRepPlanarLoop::Circle {
            center_bits: center_mm.map(f64::to_bits),
            radius_bits: radius_mm.to_bits(),
        }),
        profile => Some(ExactBRepPlanarLoop::Boundary {
            segments: exact_planar_offset_profile_from_solved(profile)?.segments,
        }),
    }
}

fn planar_offset_loop_segment_count(planar_loop: &ExactBRepPlanarLoop) -> usize {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => segments.len(),
        ExactBRepPlanarLoop::Circle { .. } => 1,
    }
}

fn planar_offset_loop_bounds_bits(planar_loop: &ExactBRepPlanarLoop) -> Option<[u64; 4]> {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => {
            Some(exact_planar_offset_profile_from_segments(segments.clone())?.bounds_bits)
        }
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            Some(
                [
                    center[0] - radius,
                    center[1] - radius,
                    center[0] + radius,
                    center[1] + radius,
                ]
                .map(f64::to_bits),
            )
        }
    }
}

fn planar_offset_loop_area_mm2(planar_loop: &ExactBRepPlanarLoop) -> Option<f64> {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => Some(f64::from_bits(
            exact_planar_offset_profile_from_segments(segments.clone())?.area_bits,
        )),
        ExactBRepPlanarLoop::Circle { radius_bits, .. } => {
            let radius = f64::from_bits(*radius_bits);
            Some(std::f64::consts::PI * radius * radius)
        }
    }
}

const PLANAR_OFFSET_CLEARANCE_TOLERANCE_MM: f64 = APPROXIMATION;
const MAX_PLANAR_OFFSET_CLEARANCE_CURVES: usize = 16_384;
const MAX_PLANAR_OFFSET_CLEARANCE_COMPARISONS: usize = 16_777_216;

#[derive(Clone, Copy)]
enum PlanarOffsetClearanceCurve {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    Arc {
        start: [f64; 2],
        end: [f64; 2],
        center: [f64; 2],
        clockwise: bool,
    },
}

fn planar_offset_loop_displacement_mm(
    planar_loop: &ExactBRepPlanarLoop,
    distance_mm: f64,
) -> Option<f64> {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => {
            exact_planar_offset_profile_from_segments(segments.clone())?
                .max_planar_offset_displacement_mm(distance_mm)
        }
        ExactBRepPlanarLoop::Circle { .. } => Some(distance_mm.abs()),
    }
}

fn planar_offset_loop_clearance_curves(
    planar_loop: &ExactBRepPlanarLoop,
) -> Option<Vec<PlanarOffsetClearanceCurve>> {
    let mut curves = Vec::new();
    match planar_loop {
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            let right = [center[0] + radius, center[1]];
            let left = [center[0] - radius, center[1]];
            curves.extend([
                PlanarOffsetClearanceCurve::Arc {
                    start: right,
                    end: left,
                    center,
                    clockwise: false,
                },
                PlanarOffsetClearanceCurve::Arc {
                    start: left,
                    end: right,
                    center,
                    clockwise: false,
                },
            ]);
        }
        ExactBRepPlanarLoop::Boundary { segments } => {
            for segment in segments {
                match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } => curves.push(PlanarOffsetClearanceCurve::Line {
                        start: start_bits.map(f64::from_bits),
                        end: end_bits.map(f64::from_bits),
                    }),
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => curves.push(PlanarOffsetClearanceCurve::Arc {
                        start: start_bits.map(f64::from_bits),
                        end: end_bits.map(f64::from_bits),
                        center: center_bits.map(f64::from_bits),
                        clockwise: *clockwise,
                    }),
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } => {
                        let mut stack = vec![(
                            [
                                start_bits.map(f64::from_bits),
                                control_1_bits.map(f64::from_bits),
                                control_2_bits.map(f64::from_bits),
                                end_bits.map(f64::from_bits),
                            ],
                            0_u8,
                        )];
                        while let Some((cubic, depth)) = stack.pop() {
                            let flatness = point_to_segment_distance(cubic[1], cubic[0], cubic[3])
                                .max(point_to_segment_distance(cubic[2], cubic[0], cubic[3]));
                            if flatness <= PLANAR_OFFSET_CLEARANCE_TOLERANCE_MM {
                                curves.push(PlanarOffsetClearanceCurve::Line {
                                    start: cubic[0],
                                    end: cubic[3],
                                });
                                if curves.len() > MAX_PLANAR_OFFSET_CLEARANCE_CURVES {
                                    return None;
                                }
                                continue;
                            }
                            if depth >= 16
                                || curves.len() + stack.len() >= MAX_PLANAR_OFFSET_CLEARANCE_CURVES
                            {
                                return None;
                            }
                            let midpoint = |left: [f64; 2], right: [f64; 2]| {
                                [(left[0] + right[0]) * 0.5, (left[1] + right[1]) * 0.5]
                            };
                            let p01 = midpoint(cubic[0], cubic[1]);
                            let p12 = midpoint(cubic[1], cubic[2]);
                            let p23 = midpoint(cubic[2], cubic[3]);
                            let p012 = midpoint(p01, p12);
                            let p123 = midpoint(p12, p23);
                            let split = midpoint(p012, p123);
                            stack.push(([split, p123, p23, cubic[3]], depth + 1));
                            stack.push(([cubic[0], p01, p012, split], depth + 1));
                        }
                    }
                }
            }
        }
    }
    Some(curves)
}

fn planar_offset_clearance_curve_start(curve: PlanarOffsetClearanceCurve) -> [f64; 2] {
    match curve {
        PlanarOffsetClearanceCurve::Line { start, .. }
        | PlanarOffsetClearanceCurve::Arc { start, .. } => start,
    }
}

fn point_in_planar_offset_clearance_curves(
    point: [f64; 2],
    curves: &[PlanarOffsetClearanceCurve],
) -> bool {
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let subtract = |left: [f64; 2], right: [f64; 2]| [left[0] - right[0], left[1] - right[1]];
    let mut winding = 0_i32;
    for curve in curves {
        match *curve {
            PlanarOffsetClearanceCurve::Line { start, end } => {
                if start[1] <= point[1]
                    && point[1] < end[1]
                    && cross(subtract(end, start), subtract(point, start)) > 0.0
                {
                    winding += 1;
                } else if end[1] <= point[1]
                    && point[1] < start[1]
                    && cross(subtract(end, start), subtract(point, start)) < 0.0
                {
                    winding -= 1;
                }
            }
            arc @ PlanarOffsetClearanceCurve::Arc {
                start,
                center,
                clockwise,
                ..
            } => {
                let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let relative_y = (point[1] - center[1]) / radius;
                if relative_y.abs() > 1.0 {
                    continue;
                }
                let angle = relative_y.clamp(-1.0, 1.0).asin();
                for candidate in [angle, std::f64::consts::PI - angle] {
                    let crossing = [
                        center[0] + radius * candidate.cos(),
                        center[1] + radius * candidate.sin(),
                    ];
                    if crossing[0] <= point[0] + PLANAR_OFFSET_CLEARANCE_TOLERANCE_MM
                        || !point_is_on_clearance_arc(crossing, arc)
                    {
                        continue;
                    }
                    let derivative = candidate.cos() * if clockwise { -1.0 } else { 1.0 };
                    if derivative > PLANAR_OFFSET_CLEARANCE_TOLERANCE_MM {
                        winding += 1;
                    } else if derivative < -PLANAR_OFFSET_CLEARANCE_TOLERANCE_MM {
                        winding -= 1;
                    }
                }
            }
        }
    }
    winding != 0
}

fn point_to_segment_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let direction = [end[0] - start[0], end[1] - start[1]];
    let length_squared = ketchup_geometry::linalg::dot2(direction, direction);
    if length_squared <= f64::EPSILON {
        return (point[0] - start[0]).hypot(point[1] - start[1]);
    }
    let parameter = (((point[0] - start[0]) * direction[0] + (point[1] - start[1]) * direction[1])
        / length_squared)
        .clamp(0.0, 1.0);
    (point[0] - start[0] - parameter * direction[0])
        .hypot(point[1] - start[1] - parameter * direction[1])
}

fn point_is_on_clearance_arc(point: [f64; 2], arc: PlanarOffsetClearanceCurve) -> bool {
    let PlanarOffsetClearanceCurve::Arc {
        start,
        end,
        center,
        clockwise,
    } = arc
    else {
        return false;
    };
    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
    let candidate = (point[1] - center[1]).atan2(point[0] - center[0]);
    directed_arc_sweep(start_angle, end_angle, clockwise)
        .is_some_and(|sweep| angle_on_directed_arc(start_angle, sweep, candidate))
}

fn point_to_clearance_arc_distance(point: [f64; 2], arc: PlanarOffsetClearanceCurve) -> f64 {
    let PlanarOffsetClearanceCurve::Arc {
        start, end, center, ..
    } = arc
    else {
        unreachable!()
    };
    let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
    let center_distance = (point[0] - center[0]).hypot(point[1] - center[1]);
    let mut distance = (point[0] - start[0])
        .hypot(point[1] - start[1])
        .min((point[0] - end[0]).hypot(point[1] - end[1]));
    if center_distance > f64::EPSILON {
        let radial = [
            center[0] + radius * (point[0] - center[0]) / center_distance,
            center[1] + radius * (point[1] - center[1]) / center_distance,
        ];
        if point_is_on_clearance_arc(radial, arc) {
            distance = distance.min((center_distance - radius).abs());
        }
    }
    distance
}

fn line_to_line_clearance(
    left_start: [f64; 2],
    left_end: [f64; 2],
    right_start: [f64; 2],
    right_end: [f64; 2],
) -> f64 {
    let left = [left_end[0] - left_start[0], left_end[1] - left_start[1]];
    let right = [right_end[0] - right_start[0], right_end[1] - right_start[1]];
    let offset = [
        right_start[0] - left_start[0],
        right_start[1] - left_start[1],
    ];
    let cross = |first: [f64; 2], second: [f64; 2]| first[0] * second[1] - first[1] * second[0];
    let denominator = cross(left, right);
    if denominator.abs() > f64::EPSILON {
        let along_left = cross(offset, right) / denominator;
        let along_right = cross(offset, left) / denominator;
        if (0.0..=1.0).contains(&along_left) && (0.0..=1.0).contains(&along_right) {
            return 0.0;
        }
    }
    point_to_segment_distance(left_start, right_start, right_end)
        .min(point_to_segment_distance(left_end, right_start, right_end))
        .min(point_to_segment_distance(right_start, left_start, left_end))
        .min(point_to_segment_distance(right_end, left_start, left_end))
}

fn line_to_arc_clearance(
    line_start: [f64; 2],
    line_end: [f64; 2],
    arc: PlanarOffsetClearanceCurve,
) -> f64 {
    let PlanarOffsetClearanceCurve::Arc {
        start, end, center, ..
    } = arc
    else {
        unreachable!()
    };
    let direction = [line_end[0] - line_start[0], line_end[1] - line_start[1]];
    let offset = [line_start[0] - center[0], line_start[1] - center[1]];
    let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
    let a = ketchup_geometry::linalg::dot2(direction, direction);
    let b = 2.0 * ketchup_geometry::linalg::dot2(offset, direction);
    let c = ketchup_geometry::linalg::dot2(offset, offset) - radius * radius;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant >= 0.0 {
        let root = discriminant.sqrt();
        if [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
            .into_iter()
            .any(|parameter| {
                (0.0..=1.0).contains(&parameter)
                    && point_is_on_clearance_arc(
                        [
                            line_start[0] + parameter * direction[0],
                            line_start[1] + parameter * direction[1],
                        ],
                        arc,
                    )
            })
        {
            return 0.0;
        }
    }
    let projection = if a > f64::EPSILON {
        ((center[0] - line_start[0]) * direction[0] + (center[1] - line_start[1]) * direction[1])
            / a
    } else {
        0.0
    }
    .clamp(0.0, 1.0);
    point_to_clearance_arc_distance(line_start, arc)
        .min(point_to_clearance_arc_distance(line_end, arc))
        .min(point_to_segment_distance(start, line_start, line_end))
        .min(point_to_segment_distance(end, line_start, line_end))
        .min(point_to_clearance_arc_distance(
            [
                line_start[0] + projection * direction[0],
                line_start[1] + projection * direction[1],
            ],
            arc,
        ))
}

fn arc_to_arc_clearance(
    left: PlanarOffsetClearanceCurve,
    right: PlanarOffsetClearanceCurve,
) -> f64 {
    let PlanarOffsetClearanceCurve::Arc {
        start: left_start,
        end: left_end,
        center: left_center,
        ..
    } = left
    else {
        unreachable!()
    };
    let PlanarOffsetClearanceCurve::Arc {
        start: right_start,
        end: right_end,
        center: right_center,
        ..
    } = right
    else {
        unreachable!()
    };
    let left_radius = (left_start[0] - left_center[0]).hypot(left_start[1] - left_center[1]);
    let right_radius = (right_start[0] - right_center[0]).hypot(right_start[1] - right_center[1]);
    let center_delta = [
        right_center[0] - left_center[0],
        right_center[1] - left_center[1],
    ];
    let center_distance = center_delta[0].hypot(center_delta[1]);
    if center_distance > f64::EPSILON
        && center_distance <= left_radius + right_radius
        && center_distance >= (left_radius - right_radius).abs()
    {
        let along = (left_radius * left_radius - right_radius * right_radius
            + center_distance * center_distance)
            / (2.0 * center_distance);
        let height = (left_radius * left_radius - along * along).max(0.0).sqrt();
        let direction = [
            center_delta[0] / center_distance,
            center_delta[1] / center_distance,
        ];
        let base = [
            left_center[0] + along * direction[0],
            left_center[1] + along * direction[1],
        ];
        let perpendicular = [-direction[1], direction[0]];
        if [
            [
                base[0] + height * perpendicular[0],
                base[1] + height * perpendicular[1],
            ],
            [
                base[0] - height * perpendicular[0],
                base[1] - height * perpendicular[1],
            ],
        ]
        .into_iter()
        .any(|point| {
            point_is_on_clearance_arc(point, left) && point_is_on_clearance_arc(point, right)
        }) {
            return 0.0;
        }
    }
    let mut distance = point_to_clearance_arc_distance(left_start, right)
        .min(point_to_clearance_arc_distance(left_end, right))
        .min(point_to_clearance_arc_distance(right_start, left))
        .min(point_to_clearance_arc_distance(right_end, left));
    if center_distance > f64::EPSILON {
        let direction = [
            center_delta[0] / center_distance,
            center_delta[1] / center_distance,
        ];
        for left_sign in [-1.0, 1.0] {
            let left_point = [
                left_center[0] + left_sign * left_radius * direction[0],
                left_center[1] + left_sign * left_radius * direction[1],
            ];
            if !point_is_on_clearance_arc(left_point, left) {
                continue;
            }
            for right_sign in [-1.0, 1.0] {
                let right_point = [
                    right_center[0] + right_sign * right_radius * direction[0],
                    right_center[1] + right_sign * right_radius * direction[1],
                ];
                if point_is_on_clearance_arc(right_point, right) {
                    distance = distance.min(
                        (left_point[0] - right_point[0]).hypot(left_point[1] - right_point[1]),
                    );
                }
            }
        }
    }
    distance
}

fn planar_offset_curves_clearance(
    left: PlanarOffsetClearanceCurve,
    right: PlanarOffsetClearanceCurve,
) -> f64 {
    match (left, right) {
        (
            PlanarOffsetClearanceCurve::Line {
                start: left_start,
                end: left_end,
            },
            PlanarOffsetClearanceCurve::Line {
                start: right_start,
                end: right_end,
            },
        ) => line_to_line_clearance(left_start, left_end, right_start, right_end),
        (
            PlanarOffsetClearanceCurve::Line { start, end },
            arc @ PlanarOffsetClearanceCurve::Arc { .. },
        )
        | (
            arc @ PlanarOffsetClearanceCurve::Arc { .. },
            PlanarOffsetClearanceCurve::Line { start, end },
        ) => line_to_arc_clearance(start, end, arc),
        (
            left @ PlanarOffsetClearanceCurve::Arc { .. },
            right @ PlanarOffsetClearanceCurve::Arc { .. },
        ) => arc_to_arc_clearance(left, right),
    }
}

fn planar_offset_clearance_curves_mm(
    left: &[PlanarOffsetClearanceCurve],
    right: &[PlanarOffsetClearanceCurve],
) -> Option<f64> {
    let clearance = left
        .iter()
        .flat_map(|left| {
            right
                .iter()
                .map(move |right| planar_offset_curves_clearance(*left, *right))
        })
        .reduce(f64::min)?
        - 2.0 * PLANAR_OFFSET_CLEARANCE_TOLERANCE_MM;
    clearance.is_finite().then_some(clearance.max(0.0))
}

fn planar_offset_loop_is_valid(planar_loop: &ExactBRepPlanarLoop, distance_mm: f64) -> bool {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => {
            let Some(profile) = exact_planar_offset_profile_from_segments(segments.clone()) else {
                return false;
            };
            let bounds = profile.bounds_bits.map(f64::from_bits);
            let Some(margin) = profile.max_planar_offset_displacement_mm(distance_mm) else {
                return false;
            };
            let output_envelope = if distance_mm > 0.0 {
                [
                    bounds[0] - margin,
                    bounds[1] - margin,
                    bounds[2] + margin,
                    bounds[3] + margin,
                ]
            } else {
                bounds
            };
            let cannot_statically_collapse = distance_mm > 0.0
                || bounds[2] - bounds[0] > 2.0 * distance_mm.abs()
                    && bounds[3] - bounds[1] > 2.0 * distance_mm.abs();
            output_envelope
                .into_iter()
                .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE_MM)
                && cannot_statically_collapse
        }
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            let output_radius = radius + distance_mm;
            (limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&radius)
                && (limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&output_radius)
                && [radius, output_radius].into_iter().all(f64::is_finite)
                && [
                    center[0] - radius,
                    center[1] - radius,
                    center[0] + radius,
                    center[1] + radius,
                    center[0] - output_radius,
                    center[1] - output_radius,
                    center[0] + output_radius,
                    center[1] + output_radius,
                ]
                .into_iter()
                .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE_MM)
        }
    }
}

pub fn accepts_planar_offset_solved_region(region: &SolvedSketchRegion, distance_mm: f64) -> bool {
    if !distance_mm.is_finite()
        || distance_mm.abs() < limits::MIN_LENGTH_MM
        || distance_mm.abs() > MAX_COORDINATE_MM
    {
        return false;
    }
    if region.holes.is_empty() {
        accepts_planar_offset_solved_profile(&region.outer, distance_mm)
    } else {
        ExactPlanarOffsetRegion::from_solved(region)
            .is_some_and(|region| region.has_valid_encoding(distance_mm))
    }
}

pub(crate) fn accepts_planar_offset_solved_profile(
    profile: &SolvedSketchRegionProfile,
    distance_mm: f64,
) -> bool {
    if !distance_mm.is_finite() || distance_mm.abs() < limits::MIN_LENGTH_MM {
        return false;
    }
    match profile {
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => {
            let output_radius = radius_mm + distance_mm;
            (limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(radius_mm)
                && (limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&output_radius)
                && [*radius_mm, output_radius].into_iter().all(|radius| {
                    [
                        center_mm[0] - radius,
                        center_mm[1] - radius,
                        center_mm[0] + radius,
                        center_mm[1] + radius,
                    ]
                    .into_iter()
                    .all(|coordinate| {
                        coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE_MM
                    })
                })
        }
        profile => {
            let Some(profile) = exact_planar_offset_profile_from_solved(profile) else {
                return false;
            };
            if distance_mm.abs() > MAX_COORDINATE_MM {
                return false;
            }
            let bounds = profile.bounds_bits.map(f64::from_bits);
            let Some(margin) = profile.max_planar_offset_displacement_mm(distance_mm) else {
                return false;
            };
            let output_envelope = if distance_mm > 0.0 {
                [
                    bounds[0] - margin,
                    bounds[1] - margin,
                    bounds[2] + margin,
                    bounds[3] + margin,
                ]
            } else {
                bounds
            };
            let minimum_displacement = distance_mm.abs();
            let cannot_statically_collapse = distance_mm > 0.0
                || bounds[2] - bounds[0] > 2.0 * minimum_displacement
                    && bounds[3] - bounds[1] > 2.0 * minimum_displacement;
            output_envelope
                .into_iter()
                .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE_MM)
                && cannot_statically_collapse
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExactProducerEvidenceContext {
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
}

impl ExactProducerEvidenceContext {
    #[must_use]
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self::from_source(
            snapshot.document_id(),
            snapshot.revision_id(),
            snapshot.canonical_digest(),
        )
    }

    #[must_use]
    pub fn from_source(
        document_id: DocumentId,
        source_revision: u64,
        source_digest: String,
    ) -> Self {
        Self {
            document_id,
            source_revision,
            source_digest,
        }
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
    }

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
}

#[derive(Clone, Debug)]
pub enum ExactProducerPlan {
    Graph(Box<ExactBRepGraph>),
    Imported(Box<ImportedExactBodySpec>),
}

#[derive(Clone, Copy)]
pub struct ExactProducerCompilation<'a> {
    snapshot: &'a Snapshot,
    context: &'a ExactProducerEvidenceContext,
    definition_id: DefinitionId,
    feature_id: FeatureId,
}

impl<'a> ExactProducerCompilation<'a> {
    pub fn from_snapshot(
        snapshot: &'a Snapshot,
        context: &'a ExactProducerEvidenceContext,
        definition_id: DefinitionId,
        feature_id: FeatureId,
    ) -> Result<Self, ExactBRepGraphError> {
        if !context.is_current(snapshot) {
            return Err(ExactBRepGraphError::InvalidGraph);
        }
        let definition = snapshot
            .definition(definition_id)
            .ok_or(ExactBRepGraphError::DefinitionNotFound(definition_id))?;
        let feature = snapshot
            .feature(feature_id)
            .ok_or(ExactBRepGraphError::FeatureNotFound(feature_id))?;
        if feature.definition_id() != definition_id
            || !definition.feature_ids().contains(&feature_id)
        {
            return Err(ExactBRepGraphError::FeatureNotFound(feature_id));
        }
        Ok(Self {
            snapshot,
            context,
            definition_id,
            feature_id,
        })
    }

    #[must_use]
    pub const fn context(&self) -> &ExactProducerEvidenceContext {
        self.context
    }

    pub fn plan(&self) -> Result<Option<ExactProducerPlan>, ExactBRepGraphError> {
        let feature = self
            .snapshot
            .feature(self.feature_id)
            .expect("producer compilation retains its validated feature");
        if let FeatureKind::ImportedExactBody(spec) = feature.kind() {
            return Ok(Some(ExactProducerPlan::Imported(Box::new(spec.clone()))));
        }
        match ExactBRepGraph::from_snapshot(self.snapshot, self.definition_id, self.feature_id) {
            Ok(graph) => Ok(Some(ExactProducerPlan::Graph(Box::new(graph)))),
            Err(
                ExactBRepGraphError::UnsupportedFeature(_)
                | ExactBRepGraphError::UnsupportedProfile(_)
                | ExactBRepGraphError::UnsolvedProfile(..),
            ) => Ok(None),
            Err(error) => Err(error),
        }
    }

    #[must_use]
    pub fn matches_reference(&self, reference: &BodySubshapeRef) -> bool {
        ExactBRepGraph::from_snapshot(self.snapshot, self.definition_id, self.feature_id)
            .is_ok_and(|graph| graph.names_evaluated_reference(reference))
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
    ExportResourceLimit,
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
    /// `error` raised because of a lower-level failure named by `cause`.
    Caused {
        error: Box<ExactProductError>,
        cause: String,
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
            Self::UnsupportedShell => {
                formatter.write_str("exact shell thickness is outside the supported envelope")
            }
            Self::EmptyModelExport => formatter.write_str("the visible exact model is empty"),
            Self::InvalidMeshExport => {
                formatter.write_str("the accepted exact tessellation contains an invalid facet")
            }
            Self::ExportResourceLimit => {
                formatter.write_str("exact mesh export exceeded its resource limit")
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
            Self::Caused { error, cause } => write!(formatter, "{error}: {cause}"),
        }
    }
}

impl std::error::Error for ExactProductError {}

impl ExactProductError {
    /// Wraps this error with the lower-level failure that caused it.
    #[must_use]
    pub fn because(self, cause: impl fmt::Display) -> Self {
        Self::Caused {
            error: Box::new(self),
            cause: cause.to_string(),
        }
    }
}

pub(crate) fn is_simple_linear_profile(segments: &[ProfileSegment], tolerance_mm: f64) -> bool {
    let points = segments
        .iter()
        .map(ProfileSegment::start_mm)
        .collect::<Vec<_>>();
    if points.iter().enumerate().any(|(index, point)| {
        points[index + 1..].iter().any(|candidate| {
            (point[0] - candidate[0]).abs() <= tolerance_mm
                && (point[1] - candidate[1]).abs() <= tolerance_mm
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
                tolerance_mm,
            ) {
                return false;
            }
        }
    }
    true
}

fn planar_line_segments_intersect(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    d: [f64; 2],
    tolerance_mm: f64,
) -> bool {
    // Signed distance of `point` from the line through the segment, so it compares with a
    // length tolerance; a degenerate segment keeps the raw cross product.
    let cross = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        let area = (end[0] - start[0]) * (point[1] - start[1])
            - (end[1] - start[1]) * (point[0] - start[0]);
        let length = (end[0] - start[0]).hypot(end[1] - start[1]);
        if length > tolerance_mm {
            area / length
        } else {
            area
        }
    };
    let on_segment = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        point[0] >= start[0].min(end[0]) - tolerance_mm
            && point[0] <= start[0].max(end[0]) + tolerance_mm
            && point[1] >= start[1].min(end[1]) - tolerance_mm
            && point[1] <= start[1].max(end[1]) + tolerance_mm
    };
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    if ((ab_c > tolerance_mm && ab_d < -tolerance_mm)
        || (ab_c < -tolerance_mm && ab_d > tolerance_mm))
        && ((cd_a > tolerance_mm && cd_b < -tolerance_mm)
            || (cd_a < -tolerance_mm && cd_b > tolerance_mm))
    {
        return true;
    }
    (ab_c.abs() <= tolerance_mm && on_segment(a, b, c))
        || (ab_d.abs() <= tolerance_mm && on_segment(a, b, d))
        || (cd_a.abs() <= tolerance_mm && on_segment(c, d, a))
        || (cd_b.abs() <= tolerance_mm && on_segment(c, d, b))
}

#[must_use]
pub fn exact_planar_offset_profile(
    segments: &[ProfileSegment],
    closed: bool,
    tolerance: TolerancePolicy,
) -> Option<ExactMixedProfile> {
    let profile = exact_mixed_profile(segments, closed, tolerance)?;
    (profile.has_only_line_segments() || profile.is_strict_convex_line_arc_profile(tolerance))
        .then_some(profile)
}

#[must_use]
pub fn accepts_sweep_segment_profile(
    segments: &[ProfileSegment],
    closed: bool,
    tolerance: TolerancePolicy,
) -> bool {
    exact_mixed_profile(segments, closed, tolerance).is_some()
        || exact_circle_profile(segments, closed)
            .is_some_and(|circle| f64::from_bits(circle.radius_bits) >= limits::MIN_LENGTH_MM)
}

pub(crate) fn exact_mixed_profile(
    segments: &[ProfileSegment],
    closed: bool,
    tolerance: TolerancePolicy,
) -> Option<ExactMixedProfile> {
    let line_only = segments
        .iter()
        .all(|segment| matches!(segment, ProfileSegment::Line { .. }));
    if !closed
        || !(2..=64).contains(&segments.len())
        || !segments
            .iter()
            .any(|segment| matches!(segment, ProfileSegment::Line { .. }))
        || (line_only
            && (segments.len() < 3 || !is_simple_linear_profile(segments, tolerance.linear_mm())))
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
                    || (radius - end_radius).abs() > ROUNDING * radius.max(end_radius).max(1.0)
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
            ProfileSegment::CubicBezier { .. } | ProfileSegment::Spline { .. } => return None,
        }
    }
    let area = signed_area.abs();
    if !area.is_finite() || area <= ROUNDING {
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
    (sweep.abs() > ROUNDING && sweep.abs() < std::f64::consts::TAU - ROUNDING).then_some(sweep)
}

fn angle_on_directed_arc(start: f64, sweep: f64, candidate: f64) -> bool {
    if sweep > 0.0 {
        (candidate - start).rem_euclid(std::f64::consts::TAU) <= sweep + ROUNDING
    } else {
        (start - candidate).rem_euclid(std::f64::consts::TAU) <= -sweep + ROUNDING
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_body_package_keeps_large_graph_storage_indirect() {
        let package_size = std::mem::size_of::<ExactBodyPackage>();
        let graph_size = std::mem::size_of::<ExactBRepGraphPackage>();
        assert!(
            package_size <= 576,
            "package={package_size}, graph={graph_size}"
        );
        assert!(graph_size <= 576);
    }

    #[test]
    fn line_arc_profile_self_contact_uses_the_given_tolerance() {
        // A slot 2 mm long and 0.1 mm wide: its two straight sides are 0.1 mm apart.
        let slot = [
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [2.0, 0.0],
            },
            ProfileSegment::CircularArc {
                start_mm: [2.0, 0.0],
                end_mm: [2.0, 0.1],
                center_mm: [2.0, 0.05],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [2.0, 0.1],
                end_mm: [0.0, 0.1],
            },
            ProfileSegment::CircularArc {
                start_mm: [0.0, 0.1],
                end_mm: [0.0, 0.0],
                center_mm: [0.0, 0.05],
                clockwise: false,
            },
        ];
        assert!(exact_planar_offset_profile(&slot, true, TolerancePolicy::default()).is_some());
        // With a document tolerance coarser than the slot width the sides touch.
        let coarse = TolerancePolicy::new(0.5).unwrap();
        assert!(exact_planar_offset_profile(&slot, true, coarse).is_none());
    }

    #[test]
    fn segment_contact_tolerance_is_a_distance() {
        // Two parallel 0.1 mm segments 0.05 mm apart do not touch at a 0.01 mm tolerance,
        // however short they are; the tolerance bounds their distance, not a cross product.
        let touch = |gap: f64, tolerance: f64| {
            planar_line_segments_intersect(
                [0.0, 0.0],
                [0.1, 0.0],
                [0.0, gap],
                [0.1, gap],
                tolerance,
            )
        };
        assert!(!touch(0.05, 0.01));
        assert!(touch(0.005, 0.01));
    }

    fn line(start: [f64; 2], end: [f64; 2]) -> ExactBRepPlanarSegment {
        ExactBRepPlanarSegment::Line {
            start_bits: start.map(f64::to_bits),
            end_bits: end.map(f64::to_bits),
        }
    }

    fn circle(center: [f64; 2], radius: f64) -> ExactBRepPlanarLoop {
        ExactBRepPlanarLoop::Circle {
            center_bits: center.map(f64::to_bits),
            radius_bits: radius.to_bits(),
        }
    }

    fn rectangle(min: [f64; 2], max: [f64; 2]) -> ExactBRepPlanarLoop {
        ExactBRepPlanarLoop::Boundary {
            segments: vec![
                line(min, [max[0], min[1]]),
                line([max[0], min[1]], max),
                line(max, [min[0], max[1]]),
                line([min[0], max[1]], min),
            ],
        }
    }

    #[test]
    fn compound_offset_region_requires_contained_disjoint_non_nested_holes() {
        let valid = ExactPlanarOffsetRegion {
            outer: rectangle([-10.0, -10.0], [10.0, 10.0]),
            holes: vec![circle([-3.0, 0.0], 1.0), circle([3.0, 0.0], 1.0)],
        };
        assert!(valid.has_valid_encoding(0.25));
        assert!(valid.has_valid_encoding(-0.25));

        let outside = ExactPlanarOffsetRegion {
            outer: valid.outer.clone(),
            holes: vec![circle([12.0, 0.0], 1.0)],
        };
        let overlapping = ExactPlanarOffsetRegion {
            outer: valid.outer.clone(),
            holes: vec![circle([0.0, 0.0], 2.0), circle([3.0, 0.0], 2.0)],
        };
        let nested = ExactPlanarOffsetRegion {
            outer: valid.outer,
            holes: vec![circle([0.0, 0.0], 2.0), circle([0.0, 0.0], 1.0)],
        };
        for invalid in [outside, overlapping, nested] {
            assert!(!invalid.has_valid_encoding(0.25));
            assert!(!invalid.has_valid_encoding(-0.25));
        }
    }
}
