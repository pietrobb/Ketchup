use crate::beam_m4ae::{BeamSlice, BeamValidationVerdict, ExactNotchedPiece, JointDrivenHalfLap};
use crate::document::{DocumentId, Snapshot};
use crate::fabrication::{FabricationProjectionEnvelope, PieceDimensionSheet, ProjectionStatus};
use crate::graph::{DerivedIdentity, sha256_hex};
use crate::prismatic::{Aabb, JointId};
use crate::validation::ValidationState;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

pub const BEAM_EXACT_PRODUCT_V1: &str = "ketchup.beam-exact-product.v1";
pub const BEAM_NOTCH_EVALUATOR_V1: &str = "ketchup.beam-notch-evaluator.v1";
pub const BEAM_NOTCH_REF_V1: &str = "ketchup.beam-notch-subshape-ref.v1";
pub const PIECE_DRAWING_V1: &str = "ketchup.piece-drawing.v1";
pub const MANUFACTURING_OPERATION_V1: &str = "ketchup.manufacturing-operation.v1";
pub const BEAM_DRAWING_SVG_V1: &str = "ketchup.beam-drawing-svg.v1";
pub const BEAM_MANUFACTURING_EXPORT_V1: &str = "ketchup.beam-manufacturing-export.v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BeamNotchFaceRole {
    Contact,
    WestWall,
    EastWall,
}

impl BeamNotchFaceRole {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::WestWall => "wall.west",
            Self::EastWall => "wall.east",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BeamNotchSpec {
    pub joint_id: JointId,
    pub feature_ordinal: u32,
    pub participant: HalfLapParticipant,
    pub removed: Aabb,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BeamExactPieceRequest {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub piece: DerivedIdentity,
    pub piece_key: String,
    pub stock: Aabb,
    pub occupied_components: Vec<Aabb>,
    pub notches: Vec<BeamNotchSpec>,
    pub expected_bounds: Aabb,
    pub expected_volume_mm3: f64,
    pub canonical_input_digest: String,
}

impl BeamExactPieceRequest {
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
            && self.canonical_input_digest == request_digest(self)
    }

    #[must_use]
    pub fn expected_face_roles(&self) -> Vec<(JointId, HalfLapParticipant, BeamNotchFaceRole)> {
        self.notches
            .iter()
            .flat_map(|notch| {
                let mut roles = vec![(
                    notch.joint_id,
                    notch.participant,
                    BeamNotchFaceRole::Contact,
                )];
                if notch.participant == HalfLapParticipant::A {
                    roles.extend([
                        (
                            notch.joint_id,
                            notch.participant,
                            BeamNotchFaceRole::WestWall,
                        ),
                        (
                            notch.joint_id,
                            notch.participant,
                            BeamNotchFaceRole::EastWall,
                        ),
                    ]);
                }
                roles
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeamWorkerFaceEvidence {
    pub joint_id: JointId,
    pub participant: HalfLapParticipant,
    pub role: BeamNotchFaceRole,
    pub face_ordinal: u32,
    pub lineage_digest: String,
    pub geometric_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BeamWorkerResult {
    pub request_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub backend: String,
    pub tolerance: String,
    pub volume_mm3: f64,
    pub bounds_mm: Aabb,
    pub topology_counts: [u32; 5],
    pub face_evidence: Vec<BeamWorkerFaceEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeamBodyResultIdentity {
    pub schema: &'static str,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub piece: DerivedIdentity,
    pub piece_key: String,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: &'static str,
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BeamExactResultKey {
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub piece: DerivedIdentity,
    pub piece_key: String,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
    pub schema: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeamNotchFaceRef {
    pub schema: &'static str,
    pub document_id: DocumentId,
    pub piece: DerivedIdentity,
    pub piece_key: String,
    pub joint_id: JointId,
    pub participant: HalfLapParticipant,
    pub role: BeamNotchFaceRole,
    pub expected_type: &'static str,
    pub expected_cardinality: u32,
    pub canonical_input_digest: String,
    pub exact_input_digest: String,
    pub result_fingerprint: String,
    pub evaluator: &'static str,
    pub backend: String,
    pub tolerance: String,
    pub lineage_digest: String,
    pub corroborating_geometry_fingerprint: String,
}

impl BeamNotchFaceRef {
    #[must_use]
    pub fn has_valid_lineage(&self) -> bool {
        self.schema == BEAM_NOTCH_REF_V1
            && self.expected_type == "planar_face"
            && self.expected_cardinality == 1
            && self.lineage_digest
                == beam_reference_lineage_digest(
                    self.document_id,
                    &self.piece_key,
                    self.joint_id,
                    self.participant,
                    self.role,
                )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeamRenderVertex {
    pub position_mm: [f64; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeamRenderTriangle {
    pub vertex_indices: [u32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct BeamExactPiecePackage {
    pub identity: BeamBodyResultIdentity,
    pub bounds_mm: Aabb,
    pub topology_counts: [u32; 5],
    pub vertices: Vec<BeamRenderVertex>,
    pub triangles: Vec<BeamRenderTriangle>,
    pub references: Vec<BeamNotchFaceRef>,
}

impl BeamExactPiecePackage {
    #[must_use]
    pub fn result_key(&self) -> BeamExactResultKey {
        BeamExactResultKey {
            document_id: self.identity.document_id,
            source_revision: self.identity.source_revision,
            source_digest: self.identity.source_digest.clone(),
            piece: self.identity.piece.clone(),
            piece_key: self.identity.piece_key.clone(),
            canonical_input_digest: self.identity.canonical_input_digest.clone(),
            exact_input_digest: self.identity.exact_input_digest.clone(),
            result_fingerprint: self.identity.result_fingerprint.clone(),
            evaluator: self.identity.evaluator.to_owned(),
            backend: self.identity.backend.clone(),
            tolerance: self.identity.tolerance.clone(),
            schema: self.identity.schema.to_owned(),
        }
    }

    #[must_use]
    pub fn has_valid_registry_evidence(&self) -> bool {
        let roles = self
            .references
            .iter()
            .map(|reference| (reference.joint_id, reference.participant, reference.role))
            .collect::<BTreeSet<_>>();
        let min = self.bounds_mm.min();
        let max = self.bounds_mm.max();
        self.identity.schema == BEAM_EXACT_PRODUCT_V1
            && self.identity.evaluator == BEAM_NOTCH_EVALUATOR_V1
            && self.identity.piece_key == piece_key(&self.identity.piece)
            && !self.identity.canonical_input_digest.is_empty()
            && !self.identity.exact_input_digest.is_empty()
            && !self.identity.result_fingerprint.is_empty()
            && !self.identity.backend.is_empty()
            && !self.identity.tolerance.is_empty()
            && !self.references.is_empty()
            && roles.len() == self.references.len()
            && self.references.iter().all(|reference| {
                reference.has_valid_lineage()
                    && reference.document_id == self.identity.document_id
                    && reference.piece == self.identity.piece
                    && reference.piece_key == self.identity.piece_key
                    && reference.canonical_input_digest == self.identity.canonical_input_digest
                    && reference.exact_input_digest == self.identity.exact_input_digest
                    && reference.result_fingerprint == self.identity.result_fingerprint
                    && reference.evaluator == self.identity.evaluator
                    && reference.backend == self.identity.backend
                    && reference.tolerance == self.identity.tolerance
                    && !reference.corroborating_geometry_fingerprint.is_empty()
            })
            && !self.vertices.is_empty()
            && self.vertices.iter().all(|vertex| {
                vertex.position_mm.iter().enumerate().all(|(axis, value)| {
                    value.is_finite() && *value >= min[axis] && *value <= max[axis]
                })
            })
            && !self.triangles.is_empty()
            && self.triangles.iter().all(|triangle| {
                triangle
                    .vertex_indices
                    .iter()
                    .all(|index| (*index as usize) < self.vertices.len())
            })
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.identity.document_id == snapshot.document_id()
            && self.identity.source_revision == snapshot.revision_id()
            && self.identity.source_digest == snapshot.canonical_digest()
            && self.has_valid_registry_evidence()
    }

    pub fn validate_for_request(&self, request: &BeamExactPieceRequest) -> Result<(), BeamM5Error> {
        let expected_roles = request
            .expected_face_roles()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let actual_roles = self
            .references
            .iter()
            .map(|reference| (reference.joint_id, reference.participant, reference.role))
            .collect::<BTreeSet<_>>();
        if self.identity.schema != BEAM_EXACT_PRODUCT_V1
            || self.identity.document_id != request.document_id
            || self.identity.source_revision != request.source_revision
            || self.identity.source_digest != request.source_digest
            || self.identity.piece != request.piece
            || self.identity.piece_key != request.piece_key
            || self.identity.canonical_input_digest != request.canonical_input_digest
            || self.identity.evaluator != BEAM_NOTCH_EVALUATOR_V1
            || self.identity.exact_input_digest.is_empty()
            || self.identity.result_fingerprint.is_empty()
            || self.identity.backend.is_empty()
            || self.identity.tolerance.is_empty()
            || self.bounds_mm != request.expected_bounds
            || expected_roles != actual_roles
            || self.references.len() != expected_roles.len()
            || self.references.iter().any(|reference| {
                !reference.has_valid_lineage()
                    || reference.document_id != request.document_id
                    || reference.piece != request.piece
                    || reference.piece_key != request.piece_key
                    || reference.canonical_input_digest != request.canonical_input_digest
                    || reference.exact_input_digest != self.identity.exact_input_digest
                    || reference.result_fingerprint != self.identity.result_fingerprint
                    || reference.backend != self.identity.backend
                    || reference.tolerance != self.identity.tolerance
                    || reference.corroborating_geometry_fingerprint.is_empty()
            })
            || self.vertices.is_empty()
            || self.triangles.is_empty()
            || self.triangles.iter().any(|triangle| {
                triangle
                    .vertex_indices
                    .iter()
                    .any(|index| *index as usize >= self.vertices.len())
            })
        {
            return Err(BeamM5Error::InvalidWorkerEvidence);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawingPolyline {
    pub stable_id: String,
    pub points_mm: Vec<[f64; 2]>,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PieceDrawingProjection {
    pub envelope: FabricationProjectionEnvelope,
    pub schema: &'static str,
    pub stable_drawing_id: String,
    pub piece: DerivedIdentity,
    pub exact_result: BeamBodyResultIdentity,
    pub view: String,
    pub outlines: Vec<DrawingPolyline>,
    pub dimensions: PieceDimensionSheet,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ManufacturingOperation {
    pub stable_operation_id: String,
    pub joint_id: JointId,
    pub participant: HalfLapParticipant,
    pub piece: DerivedIdentity,
    pub operation: &'static str,
    pub removed: Aabb,
    pub contact_face: BeamNotchFaceRef,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ManufacturingOperationProjection {
    pub envelope: FabricationProjectionEnvelope,
    pub schema: &'static str,
    pub operations: Vec<ManufacturingOperation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BeamM5Products {
    pub packages: BTreeMap<DerivedIdentity, Arc<BeamExactPiecePackage>>,
    pub drawing: PieceDrawingProjection,
    pub manufacturing: ManufacturingOperationProjection,
}

impl BeamM5Products {
    #[must_use]
    pub fn stable_reference_count(&self) -> usize {
        self.packages
            .values()
            .map(|package| package.references.len())
            .sum()
    }

    pub fn drawing_svg(&self) -> Result<Vec<u8>, BeamM5Error> {
        if self.drawing.envelope.status != ProjectionStatus::Complete {
            return Err(BeamM5Error::ExportBlocked);
        }
        let stock_outline = self
            .drawing
            .outlines
            .first()
            .ok_or(BeamM5Error::ExportBlocked)?;
        let min_x = stock_outline
            .points_mm
            .iter()
            .map(|point| point[0])
            .reduce(f64::min)
            .ok_or(BeamM5Error::ExportBlocked)?;
        let max_x = stock_outline
            .points_mm
            .iter()
            .map(|point| point[0])
            .reduce(f64::max)
            .ok_or(BeamM5Error::ExportBlocked)?;
        let min_y = stock_outline
            .points_mm
            .iter()
            .map(|point| point[1])
            .reduce(f64::min)
            .ok_or(BeamM5Error::ExportBlocked)?;
        let max_y = stock_outline
            .points_mm
            .iter()
            .map(|point| point[1])
            .reduce(f64::max)
            .ok_or(BeamM5Error::ExportBlocked)?;
        let mut svg = format!(
            "<!-- {BEAM_DRAWING_SVG_V1} {} -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\">\n",
            self.drawing.envelope.result_digest,
            format_number(min_x - 50.0),
            format_number(-(max_y + 50.0 + self.drawing.dimensions.chains.len() as f64 * 35.0)),
            format_number(max_x - min_x + 100.0),
            format_number(
                max_y - min_y + 100.0 + self.drawing.dimensions.chains.len() as f64 * 35.0,
            )
        );
        svg.push_str(
            "<g fill=\"none\" stroke=\"black\" stroke-width=\"4\" transform=\"scale(1,-1)\">\n",
        );
        for outline in &self.drawing.outlines {
            let points = outline
                .points_mm
                .iter()
                .map(|point| format!("{},{}", format_number(point[0]), format_number(point[1])))
                .collect::<Vec<_>>()
                .join(" ");
            svg.push_str(&format!(
                "<{} id=\"{}\" points=\"{}\"{} />\n",
                if outline.closed {
                    "polygon"
                } else {
                    "polyline"
                },
                outline.stable_id,
                points,
                if outline.closed {
                    " fill=\"#f4e4bc\""
                } else {
                    ""
                }
            ));
        }
        svg.push_str("</g>\n<g id=\"dimensions\" font-family=\"sans-serif\" font-size=\"32\" fill=\"black\">\n");
        for (index, chain) in self.drawing.dimensions.chains.iter().enumerate() {
            svg.push_str(&format!(
                "<text id=\"{}\" x=\"{}\" y=\"{}\">{}: {} mm</text>\n",
                chain.stable_chain_id,
                format_number(min_x),
                format_number(-(max_y + 15.0 + index as f64 * 35.0)),
                chain.axis,
                chain.grouped_labels.join(", ")
            ));
        }
        svg.push_str("</g>\n</svg>\n");
        Ok(svg.into_bytes())
    }

    pub fn manufacturing_export(&self) -> Result<Vec<u8>, BeamM5Error> {
        if self.manufacturing.envelope.status != ProjectionStatus::Complete
            || self
                .manufacturing
                .operations
                .iter()
                .any(|operation| !operation.contact_face.has_valid_lineage())
        {
            return Err(BeamM5Error::ExportBlocked);
        }
        let mut output = format!(
            "{BEAM_MANUFACTURING_EXPORT_V1}\ndocument_id={}\nsource_revision={}\nsource_digest={}\nresult_digest={}\n",
            self.manufacturing.envelope.document_id.0,
            self.manufacturing.envelope.source_revision,
            self.manufacturing.envelope.source_digest,
            self.manufacturing.envelope.result_digest
        );
        for operation in &self.manufacturing.operations {
            let min = operation.removed.min();
            let max = operation.removed.max();
            output.push_str(&format!(
                "operation={};joint={};participant={};piece={};min={},{},{};max={},{},{};contact_lineage={}\n",
                operation.stable_operation_id,
                operation.joint_id.0,
                operation.participant.token(),
                piece_key(&operation.piece),
                format_number(min[0]),
                format_number(min[1]),
                format_number(min[2]),
                format_number(max[0]),
                format_number(max[1]),
                format_number(max[2]),
                operation.contact_face.lineage_digest
            ));
        }
        Ok(output.into_bytes())
    }
}

pub fn requests_for_slice(
    snapshot: &Snapshot,
    slice: &BeamSlice,
) -> Result<Vec<BeamExactPieceRequest>, BeamM5Error> {
    if slice.revision_id != snapshot.revision_id()
        || !slice.full_bom.envelope.is_current(snapshot)
        || !slice.dimension_sheet.envelope.is_current(snapshot)
    {
        return Err(BeamM5Error::InvalidSlice);
    }
    slice
        .exact_pieces
        .iter()
        .map(|piece| request_for_piece(snapshot, piece, &slice.half_laps))
        .collect()
}

fn request_for_piece(
    snapshot: &Snapshot,
    piece: &ExactNotchedPiece,
    half_laps: &[JointDrivenHalfLap],
) -> Result<BeamExactPieceRequest, BeamM5Error> {
    let mut notches = Vec::new();
    for half_lap in half_laps {
        if half_lap.participant_a == piece.identity {
            notches.push(BeamNotchSpec {
                joint_id: half_lap.joint_id,
                feature_ordinal: half_lap.feature_ordinal,
                participant: HalfLapParticipant::A,
                removed: half_lap.participant_a_notch,
            });
        } else if half_lap.participant_b == piece.identity {
            notches.push(BeamNotchSpec {
                joint_id: half_lap.joint_id,
                feature_ordinal: half_lap.feature_ordinal,
                participant: HalfLapParticipant::B,
                removed: half_lap.participant_b_notch,
            });
        }
    }
    notches.sort_by_key(|notch| (notch.feature_ordinal, notch.joint_id));
    let notch_joint_ids = notches
        .iter()
        .map(|notch| notch.joint_id)
        .collect::<BTreeSet<_>>();
    let source_joint_ids = piece.source_joints.iter().copied().collect::<BTreeSet<_>>();
    if notches.is_empty()
        || notch_joint_ids.len() != notches.len()
        || notch_joint_ids != source_joint_ids
    {
        return Err(BeamM5Error::InvalidSlice);
    }
    let occupied_components = piece
        .geometry
        .components()
        .iter()
        .map(|component| component.bounds)
        .collect::<Vec<_>>();
    let expected_bounds = union_bounds(&occupied_components)?;
    let expected_volume_mm3 = occupied_components.iter().map(Aabb::volume).sum::<f64>();
    let mut request = BeamExactPieceRequest {
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        piece: piece.identity.clone(),
        piece_key: piece_key(&piece.identity),
        stock: piece.geometry.stock(),
        occupied_components,
        notches,
        expected_bounds,
        expected_volume_mm3,
        canonical_input_digest: String::new(),
    };
    request.canonical_input_digest = request_digest(&request);
    Ok(request)
}

pub fn build_piece_package(
    request: &BeamExactPieceRequest,
    result: BeamWorkerResult,
) -> Result<BeamExactPiecePackage, BeamM5Error> {
    let volume_tolerance = 1.0e-6_f64.max(request.expected_volume_mm3.abs() * 1.0e-10);
    let expected_roles = request
        .expected_face_roles()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let actual_roles = result
        .face_evidence
        .iter()
        .map(|evidence| (evidence.joint_id, evidence.participant, evidence.role))
        .collect::<BTreeSet<_>>();
    let face_ordinals = result
        .face_evidence
        .iter()
        .map(|evidence| evidence.face_ordinal)
        .collect::<BTreeSet<_>>();
    if result.request_digest != request.canonical_input_digest
        || result.exact_input_digest.is_empty()
        || result.result_fingerprint.is_empty()
        || result.backend.is_empty()
        || result.tolerance.is_empty()
        || !aabb_matches(result.bounds_mm, request.expected_bounds, 1.0e-6)
        || !result.volume_mm3.is_finite()
        || (result.volume_mm3 - request.expected_volume_mm3).abs() > volume_tolerance
        || result.topology_counts[4] != 1
        || result.face_evidence.len() != expected_roles.len()
        || face_ordinals.len() != result.face_evidence.len()
        || actual_roles != expected_roles
        || result.face_evidence.iter().any(|evidence| {
            evidence.face_ordinal >= result.topology_counts[2]
                || evidence.lineage_digest
                    != beam_reference_lineage_digest(
                        request.document_id,
                        &request.piece_key,
                        evidence.joint_id,
                        evidence.participant,
                        evidence.role,
                    )
                || evidence.geometric_fingerprint.is_empty()
        })
    {
        return Err(BeamM5Error::InvalidWorkerEvidence);
    }
    let (vertices, triangles) = component_mesh(&request.occupied_components)?;
    let identity = BeamBodyResultIdentity {
        schema: BEAM_EXACT_PRODUCT_V1,
        document_id: request.document_id,
        source_revision: request.source_revision,
        source_digest: request.source_digest.clone(),
        piece: request.piece.clone(),
        piece_key: request.piece_key.clone(),
        canonical_input_digest: request.canonical_input_digest.clone(),
        exact_input_digest: result.exact_input_digest.clone(),
        result_fingerprint: result.result_fingerprint.clone(),
        evaluator: BEAM_NOTCH_EVALUATOR_V1,
        backend: result.backend.clone(),
        tolerance: result.tolerance.clone(),
    };
    let references = result
        .face_evidence
        .into_iter()
        .map(|evidence| BeamNotchFaceRef {
            schema: BEAM_NOTCH_REF_V1,
            document_id: request.document_id,
            piece: request.piece.clone(),
            piece_key: request.piece_key.clone(),
            joint_id: evidence.joint_id,
            participant: evidence.participant,
            role: evidence.role,
            expected_type: "planar_face",
            expected_cardinality: 1,
            canonical_input_digest: request.canonical_input_digest.clone(),
            exact_input_digest: result.exact_input_digest.clone(),
            result_fingerprint: result.result_fingerprint.clone(),
            evaluator: BEAM_NOTCH_EVALUATOR_V1,
            backend: result.backend.clone(),
            tolerance: result.tolerance.clone(),
            lineage_digest: evidence.lineage_digest,
            corroborating_geometry_fingerprint: evidence.geometric_fingerprint,
        })
        .collect();
    let package = BeamExactPiecePackage {
        identity,
        bounds_mm: request.expected_bounds,
        topology_counts: result.topology_counts,
        vertices,
        triangles,
        references,
    };
    package.validate_for_request(request)?;
    Ok(package)
}

pub fn accept_products(
    snapshot: &Snapshot,
    slice: &BeamSlice,
    packages: Vec<BeamExactPiecePackage>,
) -> Result<BeamM5Products, BeamM5Error> {
    let requests = requests_for_slice(snapshot, slice)?;
    let packages = packages
        .into_iter()
        .map(|package| (package.identity.piece.clone(), Arc::new(package)))
        .collect::<BTreeMap<_, _>>();
    if packages.len() != requests.len() {
        return Err(BeamM5Error::InvalidWorkerEvidence);
    }
    for request in &requests {
        packages
            .get(&request.piece)
            .ok_or(BeamM5Error::InvalidWorkerEvidence)?
            .validate_for_request(request)?;
    }
    let complete = slice.validation == BeamValidationVerdict::Green
        && slice.validation_report.state == ValidationState::Passed
        && slice.validation_report.evidence_counts.exact == slice.half_laps.len()
        && slice.validation_report.evidence_counts.tolerant == 0;
    let status = if complete {
        ProjectionStatus::Complete
    } else {
        ProjectionStatus::Incomplete
    };
    let body_request = requests.first().ok_or(BeamM5Error::InvalidSlice)?;
    let body_package = packages
        .get(&body_request.piece)
        .ok_or(BeamM5Error::InvalidWorkerEvidence)?;
    let outlines = drawing_outlines(body_request)?;
    let drawing_bytes = drawing_projection_bytes(body_package, &outlines, &slice.dimension_sheet);
    let drawing = PieceDrawingProjection {
        envelope: FabricationProjectionEnvelope::new_with_evaluator(
            snapshot,
            &drawing_bytes,
            status,
            PIECE_DRAWING_V1,
        ),
        schema: PIECE_DRAWING_V1,
        stable_drawing_id: "beam-a/piece-drawing".to_owned(),
        piece: body_request.piece.clone(),
        exact_result: body_package.identity.clone(),
        view: "longitudinal-side".to_owned(),
        outlines,
        dimensions: slice.dimension_sheet.clone(),
    };
    let mut operations = Vec::new();
    for request in &requests {
        let package = packages
            .get(&request.piece)
            .ok_or(BeamM5Error::InvalidWorkerEvidence)?;
        for notch in &request.notches {
            let contact_face = package
                .references
                .iter()
                .find(|reference| {
                    reference.joint_id == notch.joint_id
                        && reference.participant == notch.participant
                        && reference.role == BeamNotchFaceRole::Contact
                })
                .cloned()
                .ok_or(BeamM5Error::InvalidWorkerEvidence)?;
            operations.push(ManufacturingOperation {
                stable_operation_id: format!(
                    "joint-{}/participant-{}/half-lap-notch",
                    notch.joint_id.0,
                    notch.participant.token()
                ),
                joint_id: notch.joint_id,
                participant: notch.participant,
                piece: request.piece.clone(),
                operation: "half-lap-notch",
                removed: notch.removed,
                contact_face,
            });
        }
    }
    operations.sort_by_key(|operation| (operation.joint_id, operation.participant));
    let manufacturing_bytes = manufacturing_projection_bytes(&operations);
    let manufacturing = ManufacturingOperationProjection {
        envelope: FabricationProjectionEnvelope::new_with_evaluator(
            snapshot,
            &manufacturing_bytes,
            status,
            MANUFACTURING_OPERATION_V1,
        ),
        schema: MANUFACTURING_OPERATION_V1,
        operations,
    };
    Ok(BeamM5Products {
        packages,
        drawing,
        manufacturing,
    })
}

#[must_use]
pub fn beam_reference_lineage_digest(
    document_id: DocumentId,
    piece_key: &str,
    joint_id: JointId,
    participant: HalfLapParticipant,
    role: BeamNotchFaceRole,
) -> String {
    let value = format!(
        "{}:{piece_key}:{}:{}:{}:planar_face",
        document_id.0,
        joint_id.0,
        participant.token(),
        role.token()
    );
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn piece_key(identity: &DerivedIdentity) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&identity.root_rule_node_id.0.to_le_bytes());
    for segment in identity.slot_path.segments() {
        bytes.extend_from_slice(&segment.producer_rule_id.0.to_le_bytes());
        push_text(&mut bytes, &segment.output_port);
        push_text(&mut bytes, &segment.semantic_key);
    }
    sha256_hex(&bytes)
}

fn request_digest(request: &BeamExactPieceRequest) -> String {
    let mut bytes = BEAM_EXACT_PRODUCT_V1.as_bytes().to_vec();
    bytes.extend_from_slice(&request.document_id.0.to_le_bytes());
    bytes.extend_from_slice(&request.source_revision.to_le_bytes());
    push_text(&mut bytes, &request.source_digest);
    push_text(&mut bytes, &request.piece_key);
    push_aabb(&mut bytes, request.stock);
    bytes.extend_from_slice(&(request.occupied_components.len() as u64).to_le_bytes());
    for component in &request.occupied_components {
        push_aabb(&mut bytes, *component);
    }
    bytes.extend_from_slice(&(request.notches.len() as u64).to_le_bytes());
    for notch in &request.notches {
        bytes.extend_from_slice(&notch.joint_id.0.to_le_bytes());
        bytes.extend_from_slice(&notch.feature_ordinal.to_le_bytes());
        bytes.push(match notch.participant {
            HalfLapParticipant::A => 0,
            HalfLapParticipant::B => 1,
        });
        push_aabb(&mut bytes, notch.removed);
    }
    sha256_hex(&bytes)
}

fn aabb_matches(left: Aabb, right: Aabb, tolerance: f64) -> bool {
    left.min()
        .into_iter()
        .chain(left.max())
        .zip(right.min().into_iter().chain(right.max()))
        .all(|(actual, expected)| (actual - expected).abs() <= tolerance)
}

fn union_bounds(components: &[Aabb]) -> Result<Aabb, BeamM5Error> {
    let first = components.first().ok_or(BeamM5Error::InvalidSlice)?;
    let min = std::array::from_fn(|axis| {
        components
            .iter()
            .map(|component| component.min()[axis])
            .fold(first.min()[axis], f64::min)
    });
    let max = std::array::from_fn(|axis| {
        components
            .iter()
            .map(|component| component.max()[axis])
            .fold(first.max()[axis], f64::max)
    });
    Aabb::bounded_volume(min, max).map_err(|_| BeamM5Error::InvalidSlice)
}

fn component_mesh(
    components: &[Aabb],
) -> Result<(Vec<BeamRenderVertex>, Vec<BeamRenderTriangle>), BeamM5Error> {
    let mut vertices = Vec::with_capacity(components.len() * 8);
    let mut triangles = Vec::with_capacity(components.len() * 12);
    let faces = [
        [0, 3, 1],
        [0, 2, 3],
        [4, 5, 7],
        [4, 7, 6],
        [0, 1, 5],
        [0, 5, 4],
        [1, 3, 7],
        [1, 7, 5],
        [2, 6, 7],
        [2, 7, 3],
        [0, 4, 6],
        [0, 6, 2],
    ];
    for component in components {
        let base = u32::try_from(vertices.len()).map_err(|_| BeamM5Error::InvalidSlice)?;
        vertices.extend(
            component
                .vertices()
                .into_iter()
                .map(|position_mm| BeamRenderVertex { position_mm }),
        );
        triangles.extend(faces.map(|indices| BeamRenderTriangle {
            vertex_indices: indices.map(|index| base + index),
        }));
    }
    Ok((vertices, triangles))
}

fn drawing_outlines(request: &BeamExactPieceRequest) -> Result<Vec<DrawingPolyline>, BeamM5Error> {
    let stock_min = request.stock.min();
    let stock_max = request.stock.max();
    let mut notches = request
        .notches
        .iter()
        .filter(|notch| notch.participant == HalfLapParticipant::A)
        .collect::<Vec<_>>();
    notches.sort_by(|left, right| left.removed.min()[0].total_cmp(&right.removed.min()[0]));
    let mut points = vec![
        [stock_min[0], stock_min[2]],
        [stock_max[0], stock_min[2]],
        [stock_max[0], stock_max[2]],
    ];
    let mut cursor = stock_max[0];
    for notch in notches.into_iter().rev() {
        let min = notch.removed.min();
        let max = notch.removed.max();
        if max[0] > cursor || min[0] >= max[0] || min[2] <= stock_min[2] || max[2] != stock_max[2] {
            return Err(BeamM5Error::InvalidSlice);
        }
        points.extend([
            [max[0], stock_max[2]],
            [max[0], min[2]],
            [min[0], min[2]],
            [min[0], stock_max[2]],
        ]);
        cursor = min[0];
    }
    points.push([stock_min[0], stock_max[2]]);
    Ok(vec![DrawingPolyline {
        stable_id: "beam-a/longitudinal-outline".to_owned(),
        points_mm: points,
        closed: true,
    }])
}

fn drawing_projection_bytes(
    package: &BeamExactPiecePackage,
    outlines: &[DrawingPolyline],
    dimensions: &PieceDimensionSheet,
) -> Vec<u8> {
    let mut bytes = PIECE_DRAWING_V1.as_bytes().to_vec();
    push_text(&mut bytes, &package.identity.result_fingerprint);
    push_text(&mut bytes, &dimensions.envelope.result_digest);
    for outline in outlines {
        push_text(&mut bytes, &outline.stable_id);
        bytes.push(u8::from(outline.closed));
        for point in &outline.points_mm {
            bytes.extend_from_slice(&point[0].to_bits().to_le_bytes());
            bytes.extend_from_slice(&point[1].to_bits().to_le_bytes());
        }
    }
    bytes
}

fn manufacturing_projection_bytes(operations: &[ManufacturingOperation]) -> Vec<u8> {
    let mut bytes = MANUFACTURING_OPERATION_V1.as_bytes().to_vec();
    for operation in operations {
        push_text(&mut bytes, &operation.stable_operation_id);
        bytes.extend_from_slice(&operation.joint_id.0.to_le_bytes());
        bytes.push(match operation.participant {
            HalfLapParticipant::A => 0,
            HalfLapParticipant::B => 1,
        });
        push_text(&mut bytes, &operation.contact_face.lineage_digest);
        push_aabb(&mut bytes, operation.removed);
    }
    bytes
}

fn push_aabb(bytes: &mut Vec<u8>, bounds: Aabb) {
    for value in bounds.min().into_iter().chain(bounds.max()) {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}

fn push_text(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn format_number(value: f64) -> String {
    let value = format!("{value:.6}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BeamM5Error {
    InvalidSlice,
    InvalidWorkerEvidence,
    ExportBlocked,
}

impl fmt::Display for BeamM5Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSlice => {
                formatter.write_str("the canonical beam slice cannot produce M5 requests")
            }
            Self::InvalidWorkerEvidence => formatter
                .write_str("the M5 worker evidence does not match the canonical beam request"),
            Self::ExportBlocked => formatter
                .write_str("M5 export is blocked by incomplete, stale, or invalid evidence"),
        }
    }
}

impl std::error::Error for BeamM5Error {}
