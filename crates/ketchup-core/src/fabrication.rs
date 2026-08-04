use crate::document::{DocumentId, Snapshot};
use crate::graph::{DerivedIdentity, sha256_hex};
use crate::validation::{EvidenceCounts, ValidationState};

pub const FABRICATION_PROJECTION_V1: &str = "ketchup.fabrication-projection.v1";
pub const BEAM_FABRICATION_EVALUATOR_V1: &str = "ketchup.beam-fabrication-evaluator.v1";

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
        Self {
            projection_schema: FABRICATION_PROJECTION_V1,
            evaluator_id: BEAM_FABRICATION_EVALUATOR_V1,
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
