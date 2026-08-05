use crate::document::{DocumentId, InstancePath, InstancePathStep, Snapshot};
use crate::exact_product::{BodyResultIdentity, BodySubshapeRef};
use crate::exact_validation::{ExactBodyParticipant, ExactValidationError};
use crate::graph::{DerivedIdentity, sha256_hex};
use crate::prismatic::TolerancePolicy;
use crate::validation::{
    EvidenceClass, EvidenceCounts, PermittedErrorDirection, TolerantEvidence, ValidationState,
};

pub const FABRICATION_PROJECTION_V1: &str = "ketchup.fabrication-projection.v1";
pub const BEAM_FABRICATION_EVALUATOR_V1: &str = "ketchup.beam-fabrication-evaluator.v1";
pub const EXACT_DIMENSION_EVALUATOR_V1: &str = "ketchup.exact-dimension-evaluator.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionStatus {
    Complete,
    Incomplete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FabricationProjectionEnvelope {
    pub projection_schema: &'static str,
    pub evaluator_id: &'static str,
    pub document_id: DocumentId,
    pub source_revision: u64,
    pub source_digest: String,
    pub result_digest: String,
    pub status: ProjectionStatus,
}

impl FabricationProjectionEnvelope {
    #[must_use]
    pub fn new(snapshot: &Snapshot, result_bytes: &[u8], status: ProjectionStatus) -> Self {
        Self::new_with_evaluator(
            snapshot,
            result_bytes,
            status,
            BEAM_FABRICATION_EVALUATOR_V1,
        )
    }

    #[must_use]
    pub fn new_with_evaluator(
        snapshot: &Snapshot,
        result_bytes: &[u8],
        status: ProjectionStatus,
        evaluator_id: &'static str,
    ) -> Self {
        Self {
            projection_schema: FABRICATION_PROJECTION_V1,
            evaluator_id,
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            result_digest: sha256_hex(result_bytes),
            status,
        }
    }

    #[must_use]
    pub fn complete(snapshot: &Snapshot, result_bytes: &[u8]) -> Self {
        Self::new(snapshot, result_bytes, ProjectionStatus::Complete)
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceDimensions {
    pub length_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FullBomRow {
    pub stable_row_id: String,
    pub definition_key: String,
    pub piece_kind: String,
    pub material_key: String,
    pub quantity: usize,
    pub dimensions: PieceDimensions,
    pub piece_identities: Vec<DerivedIdentity>,
    pub validation_state: ValidationState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FullBomProjection {
    pub envelope: FabricationProjectionEnvelope,
    pub evidence_counts: EvidenceCounts,
    pub rows: Vec<FullBomRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DimensionDatumRef {
    pub piece: DerivedIdentity,
    pub datum: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DimensionSegment {
    pub stable_segment_id: String,
    pub from: DimensionDatumRef,
    pub to: DimensionDatumRef,
    pub value_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DimensionChain {
    pub stable_chain_id: String,
    pub axis: String,
    pub segments: Vec<DimensionSegment>,
    pub grouped_labels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PieceDimensionSheet {
    pub envelope: FabricationProjectionEnvelope,
    pub stable_sheet_id: String,
    pub piece: DerivedIdentity,
    pub named_view: String,
    pub chains: Vec<DimensionChain>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactFaceDatumRef {
    pub instance_path: InstancePath,
    pub body: BodyResultIdentity,
    pub face: BodySubshapeRef,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactDimensionProjection {
    pub envelope: FabricationProjectionEnvelope,
    pub stable_dimension_id: String,
    pub from: ExactFaceDatumRef,
    pub to: ExactFaceDatumRef,
    pub axis: usize,
    pub value_mm: f64,
    pub evidence_class: EvidenceClass,
}

pub fn exact_parallel_face_dimension(
    snapshot: &Snapshot,
    stable_dimension_id: impl Into<String>,
    from_body: &ExactBodyParticipant,
    from_face: &BodySubshapeRef,
    to_body: &ExactBodyParticipant,
    to_face: &BodySubshapeRef,
    tolerance: TolerancePolicy,
) -> Result<ExactDimensionProjection, ExactValidationError> {
    let stable_dimension_id = stable_dimension_id.into();
    if stable_dimension_id.trim().is_empty() {
        return Err(ExactValidationError::InvalidFaceReference);
    }
    let from_plane = from_body.face_plane(from_face)?;
    let to_plane = to_body.face_plane(to_face)?;
    if from_plane.axis != to_plane.axis {
        return Err(ExactValidationError::InvalidFaceReference);
    }
    let value_mm = (to_plane.coordinate_mm - from_plane.coordinate_mm).abs();
    if !value_mm.is_finite() {
        return Err(ExactValidationError::InvalidFaceReference);
    }
    let evidence_class = EvidenceClass::weakest(
        [&from_body.evidence_class, &to_body.evidence_class],
        TolerantEvidence::new(
            tolerance.epsilon_mm(),
            EXACT_DIMENSION_EVALUATOR_V1,
            PermittedErrorDirection::BidirectionalBounded,
        )
        .expect("the exact-dimension tolerance and method identity are valid"),
    );
    let from = ExactFaceDatumRef {
        instance_path: from_body.instance_path.clone(),
        body: from_body.result_identity.clone(),
        face: from_face.clone(),
    };
    let to = ExactFaceDatumRef {
        instance_path: to_body.instance_path.clone(),
        body: to_body.result_identity.clone(),
        face: to_face.clone(),
    };
    let mut result_bytes = Vec::new();
    push_projection_bytes(&mut result_bytes, EXACT_DIMENSION_EVALUATOR_V1.as_bytes());
    push_projection_bytes(&mut result_bytes, stable_dimension_id.as_bytes());
    push_projection_path(&mut result_bytes, &from.instance_path);
    push_projection_body(&mut result_bytes, &from.body);
    push_projection_bytes(&mut result_bytes, from.face.lineage_digest.as_bytes());
    push_projection_path(&mut result_bytes, &to.instance_path);
    push_projection_body(&mut result_bytes, &to.body);
    push_projection_bytes(&mut result_bytes, to.face.lineage_digest.as_bytes());
    result_bytes.extend_from_slice(&value_mm.to_bits().to_le_bytes());
    push_projection_evidence(&mut result_bytes, &evidence_class);
    Ok(ExactDimensionProjection {
        envelope: FabricationProjectionEnvelope::new_with_evaluator(
            snapshot,
            &result_bytes,
            ProjectionStatus::Complete,
            EXACT_DIMENSION_EVALUATOR_V1,
        ),
        stable_dimension_id,
        from,
        to,
        axis: from_plane.axis,
        value_mm,
        evidence_class,
    })
}

fn push_projection_path(output: &mut Vec<u8>, path: &InstancePath) {
    output.extend_from_slice(&path.root_occurrence().0.to_le_bytes());
    output.extend_from_slice(&(path.steps().len() as u64).to_le_bytes());
    for step in path.steps() {
        let (tag, id) = match step {
            InstancePathStep::Group(id) => (0, id.0),
            InstancePathStep::Occurrence(id) => (1, id.0),
        };
        output.push(tag);
        output.extend_from_slice(&id.to_le_bytes());
    }
}

fn push_projection_body(output: &mut Vec<u8>, body: &BodyResultIdentity) {
    push_projection_bytes(output, body.schema.as_bytes());
    output.extend_from_slice(&body.document_id.0.to_le_bytes());
    output.extend_from_slice(&body.source_revision.to_le_bytes());
    push_projection_bytes(output, body.source_digest.as_bytes());
    output.extend_from_slice(&body.definition_id.0.to_le_bytes());
    output.extend_from_slice(&body.profile_feature_id.0.to_le_bytes());
    output.extend_from_slice(&body.extrusion_feature_id.0.to_le_bytes());
    output.extend_from_slice(&body.producer_feature_id.0.to_le_bytes());
    push_projection_bytes(output, body.canonical_input_digest.as_bytes());
    push_projection_bytes(output, body.exact_input_digest.as_bytes());
    push_projection_bytes(output, body.result_fingerprint.as_bytes());
    push_projection_bytes(output, body.evaluator.as_bytes());
    push_projection_bytes(output, body.backend.as_bytes());
    push_projection_bytes(output, body.tolerance.as_bytes());
}

fn push_projection_evidence(output: &mut Vec<u8>, evidence: &EvidenceClass) {
    match evidence {
        EvidenceClass::Exact => output.push(0),
        EvidenceClass::Tolerant(evidence) => {
            output.push(1);
            output.extend_from_slice(&evidence.applied_threshold_mm().to_bits().to_le_bytes());
            push_projection_bytes(output, evidence.method_identity.as_bytes());
            output.push(match evidence.permitted_error_direction {
                PermittedErrorDirection::FalsePositiveOnly => 0,
                PermittedErrorDirection::FalseNegativeOnly => 1,
                PermittedErrorDirection::BidirectionalBounded => 2,
            });
        }
    }
}

fn push_projection_bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value);
}
