use crate::document::{
    BooleanOperation, DefinitionId, DocumentId, FeatureId, FeatureKind, InstancePath,
    InstancePathStep, OccurrenceId, Snapshot, SpatialPathSegment, Transform, WeldmentJointPolicy,
    WeldmentJointPrimary,
};
use crate::exact_brep_graph::{
    ExactBRepBooleanOperation, ExactBRepGraph, ExactBRepLinearInterval, ExactBRepOperation,
    ExactBRepPlanarGeometry, ExactBRepPlanarSegment, ExactBRepProfile,
};
use crate::exact_product::{
    BodyResultIdentity, BodySubshapeRef, ExactBodyPackage, ExactResultRegistry,
};
use crate::exact_validation::{
    ExactBodyParticipant, ExactValidationError, GENERAL_BODY_VALIDATOR_CONTRACT_V1,
    GENERAL_BODY_VALIDATOR_INPUT_V1, GeneralBodyParticipant, GeneralBodySource,
    GeneralBodyValidationError, GeneralClearanceCase, general_body_input_bytes,
};
use crate::graph::{DerivedIdentity, sha256_hex};
use crate::joinery::{DowelHole, project_dowel_joint_contract};
use crate::prismatic::TolerancePolicy;
use crate::validation::{
    EvidenceClass, EvidenceCounts, PermittedErrorDirection, TolerantEvidence, ValidationReport,
    ValidationState,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub mod production;

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

pub const GENERAL_FABRICATION_EVALUATOR_V5: &str = "ketchup.general-fabrication-evaluator.v5";
pub const FABRICATION_ROLE_DIMENSION_V1: &str = "ketchup.fabrication-role.v1";
pub const TIMBER_MEMBER_ROLE_V1: &str = "fabrication.timber-member.v1";
pub const MANUFACTURED_ITEM_ROLE_V1: &str = "fabrication.manufactured-item.v1";
pub const PURCHASED_ITEM_ROLE_V1: &str = "fabrication.purchased-item.v1";
pub const MATERIAL_DIMENSION_V1: &str = "ketchup.material.v1";
pub const TIMBER_MATERIAL_V1: &str = "ketchup.material.timber.unspecified.v1";
pub const UNSPECIFIED_MATERIAL_V1: &str = "ketchup.material.unspecified.v1";
pub const GENERAL_BOM_EXPORT_V2: &str = "ketchup.general-bom-export.v2";
pub const GENERAL_DRAWING_SVG_V3: &str = "ketchup.general-drawing-svg.v3";
pub const GENERAL_MANUFACTURING_EXPORT_V2: &str = "ketchup.general-manufacturing-export.v2";
pub const WOODWOP_MPR_4_0_DRILL_EXPORT_V1: &str = "ketchup.woodwop-mpr-4.0-drill-export.v1";
pub const WOODWOP_MPR_4_0_MACHINING_EXPORT_V2: &str = "ketchup.woodwop-mpr-4.0-machining-export.v2";
pub const HOMAG_BHX_PRODUCTION_PACKAGE_V1: &str = "ketchup.homag-bhx-production-package.v1";
pub const HOMAG_CODE128_LABEL_V1: &str = "ketchup.homag-code128-label.v1";
pub const WELDMENT_CUT_LIST_EXPORT_V1: &str = "ketchup.weldment-cut-list-export.v1";
pub const WELDMENT_DRAWING_SVG_V1: &str = "ketchup.weldment-drawing-svg.v1";
pub const WELDMENT_FABRICATION_EVALUATOR_V1: &str = "ketchup.weldment-fabrication-evaluator.v1";
pub const BTLX_2_3_1_VERSION: &str = "2.3.1";
pub const BTLX_2_3_1_SCHEMA_URL: &str = "https://www.design2machine.com/btlx/BTLx_2_3_1.xsd";
pub const BTLX_2_3_1_SCHEMA_SHA256: &str =
    "208848116af3b43c189156610d3b82f6f86ea2afa7d09bc85a15876cc91cf1c6";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WoodwopMprOptions {
    pub vertical_pocket_tool_number: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HomagProductionProgram {
    pub schema: &'static str,
    pub program_name: String,
    pub instance_path: InstancePath,
    pub mpr: Vec<u8>,
    pub barcode_svg: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BtlxProfileProcessingRequest {
    PortableFreeContour,
    EdgeSawCutsThenMillContour { intermediate_saw_cuts: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BtlxExportOptions {
    pub profile_processing_request: BtlxProfileProcessingRequest,
}

impl Default for BtlxExportOptions {
    fn default() -> Self {
        Self {
            profile_processing_request: BtlxProfileProcessingRequest::EdgeSawCutsThenMillContour {
                intermediate_saw_cuts: 0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralFabricationError {
    ValidationBindingMismatch,
    FabricationRoleDimensionMissing,
    FabricationRoleDimensionAmbiguous,
    TimberMemberRoleMissing,
    TimberMemberRoleAmbiguous,
    MaterialDimensionAmbiguous,
    InvalidBomMetadata,
    UnsupportedOrUnavailableGeometry,
    InvalidGeometry,
    InvalidWeldmentGeometry,
    NoSupportedGeometry,
    ExportBlocked,
    BtlxProfileRequestUnsupported,
}

impl fmt::Display for GeneralFabricationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ValidationBindingMismatch => formatter.write_str(
                "general fabrication requires current complete general-body validation coverage",
            ),
            Self::FabricationRoleDimensionMissing => {
                formatter.write_str("the fabrication role dimension is missing")
            }
            Self::FabricationRoleDimensionAmbiguous => {
                formatter.write_str("the fabrication role dimension is ambiguous")
            }
            Self::TimberMemberRoleMissing => {
                formatter.write_str("the timber-member fabrication role is missing")
            }
            Self::TimberMemberRoleAmbiguous => {
                formatter.write_str("the timber-member fabrication role is ambiguous")
            }
            Self::MaterialDimensionAmbiguous => {
                formatter.write_str("the BOM material dimension is ambiguous")
            }
            Self::InvalidBomMetadata => {
                formatter.write_str("a BOM role or material token is invalid")
            }
            Self::UnsupportedOrUnavailableGeometry => formatter.write_str(
                "a visible geometry-bearing occurrence has unsupported or unavailable evidence",
            ),
            Self::InvalidGeometry => {
                formatter.write_str("accepted fabrication geometry has invalid local bounds")
            }
            Self::InvalidWeldmentGeometry => formatter.write_str(
                "weldment cut lists require supported straight members and unambiguous joints",
            ),
            Self::NoSupportedGeometry => {
                formatter.write_str("the document contains no supported visible body geometry")
            }
            Self::ExportBlocked => formatter.write_str(
                "fabrication export is incomplete, stale, invalid, or lacks manufacturing semantics",
            ),
            Self::BtlxProfileRequestUnsupported => formatter.write_str(
                "the requested BTLx profile processing strategy cannot represent this contour",
            ),
        }
    }
}

impl std::error::Error for GeneralFabricationError {}

impl From<GeneralBodyValidationError> for GeneralFabricationError {
    fn from(_: GeneralBodyValidationError) -> Self {
        Self::UnsupportedOrUnavailableGeometry
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GeneralBomItemKind {
    Timber,
    Manufactured,
    Purchased,
}

impl GeneralBomItemKind {
    const fn token(self) -> &'static str {
        match self {
            Self::Timber => "manufactured-timber",
            Self::Manufactured => "manufactured",
            Self::Purchased => "purchased",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralBomRow {
    pub stable_row_id: String,
    pub position: usize,
    pub definition_id: DefinitionId,
    pub source: GeneralBodySource,
    pub item_kind: GeneralBomItemKind,
    pub material_key: String,
    pub quantity: usize,
    pub dimensions: PieceDimensions,
    pub instances: Vec<InstancePath>,
    pub evidence_class: EvidenceClass,
    pub validation_state: ValidationState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralBomProjection {
    pub envelope: FabricationProjectionEnvelope,
    pub evidence_counts: EvidenceCounts,
    pub rows: Vec<GeneralBomRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralDrawingView {
    pub stable_view_id: String,
    pub name: &'static str,
    pub horizontal_axis: &'static str,
    pub vertical_axis: &'static str,
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralDimensionCallout {
    pub stable_dimension_id: String,
    pub axis: &'static str,
    pub value_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralPieceDrawing {
    pub stable_drawing_id: String,
    pub bom_row_id: String,
    pub position: usize,
    pub definition_id: DefinitionId,
    pub source: GeneralBodySource,
    pub item_kind: GeneralBomItemKind,
    pub material_key: String,
    pub quantity: usize,
    pub instances: Vec<InstancePath>,
    pub projection_method: &'static str,
    pub views: Vec<GeneralDrawingView>,
    pub dimensions: Vec<GeneralDimensionCallout>,
    pub machining_operations: Vec<GeneralManufacturingOperation>,
    pub evidence_class: EvidenceClass,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralDrawingProjection {
    pub envelope: FabricationProjectionEnvelope,
    pub validation_state: ValidationState,
    pub drawings: Vec<GeneralPieceDrawing>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralManufacturingKind {
    Stock,
    ThroughCut,
    ProfileCut,
    CircularDrill,
    BooleanCut,
}

impl GeneralManufacturingKind {
    const fn token(self) -> &'static str {
        match self {
            Self::Stock => "stock",
            Self::ThroughCut => "through-cut",
            Self::ProfileCut => "profile-cut",
            Self::CircularDrill => "circular-drill",
            Self::BooleanCut => "boolean-cut",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct GeneralMachiningFrame {
    pub origin_mm: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub normal: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneralMachiningSegment {
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

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneralMachiningGeometry {
    TimberStock {
        frame: GeneralMachiningFrame,
        cross_section: Vec<GeneralMachiningSegment>,
        start_mm: [f64; 3],
        length_axis: [f64; 3],
        length_mm: f64,
        cross_section_width_mm: f64,
        cross_section_height_mm: f64,
    },
    ProfileCut {
        frame: GeneralMachiningFrame,
        segments: Vec<GeneralMachiningSegment>,
        start_mm: f64,
        end_mm: f64,
    },
    CircularDrill {
        frame: GeneralMachiningFrame,
        center_mm: [f64; 2],
        diameter_mm: f64,
        start_mm: f64,
        end_mm: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralManufacturingOperation {
    pub stable_operation_id: String,
    pub definition_id: DefinitionId,
    pub producer_feature_id: FeatureId,
    pub kind: GeneralManufacturingKind,
    pub semantic_inputs: Vec<FeatureId>,
    pub frame: &'static str,
    pub bounds: PieceDimensions,
    pub machining: GeneralMachiningGeometry,
    pub source: crate::exact_product::ExactResultKey,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralManufacturingProjection {
    pub envelope: FabricationProjectionEnvelope,
    pub validation_state: ValidationState,
    pub operations: Vec<GeneralManufacturingOperation>,
    pub unresolved_sources: Vec<GeneralBodySource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WeldmentCutTreatment {
    Square,
    Butt,
    Miter,
}

impl WeldmentCutTreatment {
    const fn token(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Butt => "butt",
            Self::Miter => "miter",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeldmentEndCut {
    pub treatment: WeldmentCutTreatment,
    pub angle_degrees: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeldmentCutListRow {
    pub stable_row_id: String,
    pub position: usize,
    pub definition_id: DefinitionId,
    pub member_feature_id: FeatureId,
    pub profile_feature_id: FeatureId,
    pub material_key: String,
    pub quantity: usize,
    pub centerline_length_mm: f64,
    pub orientation_degrees: f64,
    pub start_cut: WeldmentEndCut,
    pub end_cut: WeldmentEndCut,
    pub instances: Vec<InstancePath>,
    pub evidence_class: EvidenceClass,
    pub validation_state: ValidationState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeldmentCutListProjection {
    pub cut_list_envelope: FabricationProjectionEnvelope,
    pub drawing_envelope: FabricationProjectionEnvelope,
    pub validation_state: ValidationState,
    pub rows: Vec<WeldmentCutListRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralFabricationProjection {
    pub bom: GeneralBomProjection,
    pub drawings: GeneralDrawingProjection,
    pub manufacturing: GeneralManufacturingProjection,
    pub weldment: Option<WeldmentCutListProjection>,
}

impl WeldmentCutListProjection {
    pub fn cut_list_export(&self, snapshot: &Snapshot) -> Result<Vec<u8>, GeneralFabricationError> {
        let cut_list_bytes = weldment_cut_list_bytes(&self.rows, self.validation_state);
        let drawing_bytes = weldment_drawing_bytes(
            &self.rows,
            &self.cut_list_envelope.result_digest,
            self.validation_state,
        );
        if self.rows.is_empty()
            || self.validation_state != ValidationState::Passed
            || self.cut_list_envelope.status != ProjectionStatus::Complete
            || self.drawing_envelope.status != ProjectionStatus::Complete
            || !weldment_envelope_is_current(&self.cut_list_envelope, snapshot)
            || !weldment_envelope_is_current(&self.drawing_envelope, snapshot)
            || self.cut_list_envelope.result_digest != sha256_hex(&cut_list_bytes)
            || self.drawing_envelope.result_digest != sha256_hex(&drawing_bytes)
            || self.rows.iter().any(|row| {
                row.validation_state != ValidationState::Passed
                    || row.quantity == 0
                    || row.quantity != row.instances.len()
            })
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let mut output = format!(
            "{WELDMENT_CUT_LIST_EXPORT_V1}\ndocument_id={}\nsource_revision={}\nsource_digest={}\nresult_digest={}\ndrawing_result_digest={}\n",
            self.cut_list_envelope.document_id.0,
            self.cut_list_envelope.source_revision,
            self.cut_list_envelope.source_digest,
            self.cut_list_envelope.result_digest,
            self.drawing_envelope.result_digest,
        );
        for row in &self.rows {
            output.push_str(&format!(
                "row={};position={};definition={};member={};profile={};quantity={};length_mm={};orientation_degrees={};start={}:{};end={}:{};material={};evidence={}\n",
                row.stable_row_id,
                row.position,
                row.definition_id.0,
                row.member_feature_id.0,
                row.profile_feature_id.0,
                row.quantity,
                format_number(row.centerline_length_mm),
                format_number(row.orientation_degrees),
                row.start_cut.treatment.token(),
                format_number(row.start_cut.angle_degrees),
                row.end_cut.treatment.token(),
                format_number(row.end_cut.angle_degrees),
                row.material_key,
                evidence_token(&row.evidence_class),
            ));
        }
        Ok(output.into_bytes())
    }

    pub fn drawing_svg(&self, snapshot: &Snapshot) -> Result<Vec<u8>, GeneralFabricationError> {
        self.cut_list_export(snapshot)?;
        let drawing_bytes = weldment_drawing_bytes(
            &self.rows,
            &self.cut_list_envelope.result_digest,
            self.validation_state,
        );
        if self.drawing_envelope.result_digest != sha256_hex(&drawing_bytes) {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let height = 45usize
            .checked_add(
                self.rows
                    .len()
                    .checked_mul(45)
                    .ok_or(GeneralFabricationError::ExportBlocked)?,
            )
            .ok_or(GeneralFabricationError::ExportBlocked)?;
        let mut output = format!(
            "<!-- {WELDMENT_DRAWING_SVG_V1} cut_list={} drawing={} -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1200 {height}\">\n",
            self.cut_list_envelope.result_digest, self.drawing_envelope.result_digest,
        );
        for (index, row) in self.rows.iter().enumerate() {
            let y = 35 + index * 45;
            output.push_str(&format!(
                "<g id=\"{}/drawing\"><line x1=\"25\" y1=\"{y}\" x2=\"{}\" y2=\"{y}\" stroke=\"black\"/><text x=\"25\" y=\"{}\">position {}, member {}, length {} mm, start {} {} deg, end {} {} deg, material {}</text></g>\n",
                row.stable_row_id,
                25.0 + row.centerline_length_mm.min(400.0),
                y + 20,
                row.position,
                row.member_feature_id.0,
                format_number(row.centerline_length_mm),
                row.start_cut.treatment.token(),
                format_number(row.start_cut.angle_degrees),
                row.end_cut.treatment.token(),
                format_number(row.end_cut.angle_degrees),
                xml_escape(&row.material_key),
            ));
        }
        output.push_str("</svg>\n");
        Ok(output.into_bytes())
    }
}

impl GeneralFabricationProjection {
    pub fn bom_export(&self, snapshot: &Snapshot) -> Result<Vec<u8>, GeneralFabricationError> {
        let result_bytes = general_bom_bytes(&self.bom.rows, self.bom.evidence_counts);
        if self.bom.envelope.status != ProjectionStatus::Complete
            || !general_envelope_is_current(&self.bom.envelope, snapshot)
            || self.bom.envelope.result_digest != sha256_hex(&result_bytes)
            || self
                .bom
                .rows
                .iter()
                .any(|row| row.validation_state != ValidationState::Passed)
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let mut output = format!(
            "{GENERAL_BOM_EXPORT_V2}\ndocument_id={}\nsource_revision={}\nsource_digest={}\nresult_digest={}\n",
            self.bom.envelope.document_id.0,
            self.bom.envelope.source_revision,
            self.bom.envelope.source_digest,
            self.bom.envelope.result_digest
        );
        for row in &self.bom.rows {
            output.push_str(&format!(
                "row={};position={};definition={};kind={};quantity={};length_mm={};width_mm={};height_mm={};material={};evidence={}\n",
                row.stable_row_id,
                row.position,
                row.definition_id.0,
                row.item_kind.token(),
                row.quantity,
                format_number(row.dimensions.length_mm),
                format_number(row.dimensions.width_mm),
                format_number(row.dimensions.height_mm),
                row.material_key,
                evidence_token(&row.evidence_class)
            ));
        }
        Ok(output.into_bytes())
    }

    pub fn drawing_svg(&self, snapshot: &Snapshot) -> Result<Vec<u8>, GeneralFabricationError> {
        self.bom_export(snapshot)?;
        if self.drawings.drawings.len() != self.bom.rows.len()
            || self
                .drawings
                .drawings
                .iter()
                .zip(&self.bom.rows)
                .any(|(drawing, row)| {
                    drawing.stable_drawing_id != format!("{}/drawing", row.stable_row_id)
                        || drawing.bom_row_id != row.stable_row_id
                        || drawing.position != row.position
                        || drawing.definition_id != row.definition_id
                        || drawing.source != row.source
                        || drawing.item_kind != row.item_kind
                        || drawing.material_key != row.material_key
                        || drawing.quantity != row.quantity
                        || drawing.instances != row.instances
                        || drawing.evidence_class != row.evidence_class
                })
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let result_bytes =
            general_drawing_bytes(&self.drawings.drawings, self.drawings.validation_state);
        let manufacturing_bytes = general_manufacturing_bytes(
            &self.manufacturing.operations,
            &self.manufacturing.unresolved_sources,
            self.manufacturing.validation_state,
        );
        let drawing_operations = self
            .drawings
            .drawings
            .iter()
            .flat_map(|drawing| drawing.machining_operations.iter())
            .collect::<Vec<_>>();
        let manufacturing_operations = self.manufacturing.operations.iter().collect::<Vec<_>>();
        if self.drawings.envelope.status != ProjectionStatus::Complete
            || !general_envelope_is_current(&self.drawings.envelope, snapshot)
            || self.drawings.envelope.result_digest != sha256_hex(&result_bytes)
            || !general_envelope_is_current(&self.manufacturing.envelope, snapshot)
            || self.manufacturing.envelope.result_digest != sha256_hex(&manufacturing_bytes)
            || drawing_operations != manufacturing_operations
            || self.drawings.validation_state != ValidationState::Passed
            || self.drawings.drawings.is_empty()
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let mut sheet_width = 100.0_f64;
        let mut sheet_height = 40.0;
        for drawing in &self.drawings.drawings {
            sheet_width = drawing
                .views
                .iter()
                .map(|view| view.width_mm)
                .fold(sheet_width, f64::max);
            sheet_height += drawing
                .views
                .iter()
                .map(|view| view.height_mm + 55.0)
                .sum::<f64>()
                + 90.0;
            for operation in drawing
                .machining_operations
                .iter()
                .filter(|operation| operation.kind != GeneralManufacturingKind::Stock)
            {
                let (minimum, maximum) = machining_detail_bounds(&operation.machining)
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                sheet_width = sheet_width.max(maximum[0] - minimum[0]);
                sheet_height += maximum[1] - minimum[1] + 55.0;
            }
        }
        sheet_width += 80.0;
        let mut svg = format!(
            "<!-- {GENERAL_DRAWING_SVG_V3} bom={} drawing={} manufacturing={} -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\">\n",
            self.bom.envelope.result_digest,
            self.drawings.envelope.result_digest,
            self.manufacturing.envelope.result_digest,
            format_number(sheet_width),
            format_number(sheet_height)
        );
        let mut y = 25.0;
        for drawing in &self.drawings.drawings {
            svg.push_str(&format!(
                "<g id=\"{}\" fill=\"none\" stroke=\"black\">\n",
                drawing.stable_drawing_id
            ));
            svg.push_str(&format!(
                "<text id=\"{}/bom-reference\" x=\"25\" y=\"{}\" fill=\"black\" stroke=\"none\">position: {}, quantity: {}, kind: {}, material: {}</text>\n",
                drawing.stable_drawing_id,
                format_number(y),
                drawing.position,
                drawing.quantity,
                drawing.item_kind.token(),
                drawing.material_key
            ));
            y += 30.0;
            for view in &drawing.views {
                svg.push_str(&format!(
                    "<rect id=\"{}\" x=\"25\" y=\"{}\" width=\"{}\" height=\"{}\" />\n<text x=\"25\" y=\"{}\" fill=\"black\" stroke=\"none\">{} ({} × {})</text>\n",
                    view.stable_view_id,
                    format_number(y),
                    format_number(view.width_mm),
                    format_number(view.height_mm),
                    format_number(y + view.height_mm + 20.0),
                    view.name,
                    view.horizontal_axis,
                    view.vertical_axis
                ));
                y += view.height_mm + 55.0;
            }
            svg.push_str(&format!(
                "<text id=\"{}/overall-dimensions\" x=\"25\" y=\"{}\" fill=\"black\" stroke=\"none\">overall: {}</text>\n",
                drawing.stable_drawing_id,
                format_number(y),
                drawing
                    .dimensions
                    .iter()
                    .map(|dimension| format!(
                        "{}={} mm",
                        dimension.axis,
                        format_number(dimension.value_mm)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            y += 35.0;
            for operation in drawing
                .machining_operations
                .iter()
                .filter(|operation| operation.kind != GeneralManufacturingKind::Stock)
            {
                let (minimum, maximum) = machining_detail_bounds(&operation.machining)
                    .expect("machining detail bounds were validated during sheet sizing");
                append_machining_detail(&mut svg, operation, minimum, maximum, 25.0, y);
                y += maximum[1] - minimum[1] + 55.0;
            }
            svg.push_str("</g>\n");
            y += 25.0;
        }
        svg.push_str("</svg>\n");
        Ok(svg.into_bytes())
    }

    pub fn manufacturing_export(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<u8>, GeneralFabricationError> {
        let result_bytes = general_manufacturing_bytes(
            &self.manufacturing.operations,
            &self.manufacturing.unresolved_sources,
            self.manufacturing.validation_state,
        );
        if self.manufacturing.envelope.status != ProjectionStatus::Complete
            || !general_envelope_is_current(&self.manufacturing.envelope, snapshot)
            || self.manufacturing.envelope.result_digest != sha256_hex(&result_bytes)
            || self.manufacturing.validation_state != ValidationState::Passed
            || !self.manufacturing.unresolved_sources.is_empty()
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let mut output = format!(
            "{GENERAL_MANUFACTURING_EXPORT_V2}\ndocument_id={}\nsource_revision={}\nsource_digest={}\nresult_digest={}\n",
            self.manufacturing.envelope.document_id.0,
            self.manufacturing.envelope.source_revision,
            self.manufacturing.envelope.source_digest,
            self.manufacturing.envelope.result_digest
        );
        for operation in &self.manufacturing.operations {
            output.push_str(&format!(
                "operation={};definition={};producer={};kind={};frame={};inputs={};length_mm={};width_mm={};height_mm={};machining={};result_fingerprint={}\n",
                operation.stable_operation_id,
                operation.definition_id.0,
                operation.producer_feature_id.0,
                operation.kind.token(),
                operation.frame,
                operation
                    .semantic_inputs
                    .iter()
                    .map(|feature| feature.0.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                format_number(operation.bounds.length_mm),
                format_number(operation.bounds.width_mm),
                format_number(operation.bounds.height_mm),
                machining_token(&operation.machining),
                operation.source.result_fingerprint
            ));
        }
        Ok(output.into_bytes())
    }

    pub fn woodwop_mpr_4_0_drill_export(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<u8>, GeneralFabricationError> {
        self.woodwop_mpr_4_0_machining_export(snapshot, WoodwopMprOptions::default())
    }

    pub fn woodwop_mpr_4_0_machining_export(
        &self,
        snapshot: &Snapshot,
        options: WoodwopMprOptions,
    ) -> Result<Vec<u8>, GeneralFabricationError> {
        self.bom_export(snapshot)?;
        self.manufacturing_export(snapshot)?;
        let [row] = self.bom.rows.as_slice() else {
            return Err(GeneralFabricationError::ExportBlocked);
        };
        if row.quantity == 0
            || row.quantity != row.instances.len()
            || row.item_kind == GeneralBomItemKind::Purchased
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let GeneralBodySource::Exact(row_source) = &row.source else {
            return Err(GeneralFabricationError::ExportBlocked);
        };
        let matching = self
            .manufacturing
            .operations
            .iter()
            .filter(|operation| {
                operation.definition_id == row.definition_id && operation.source == *row_source
            })
            .collect::<Vec<_>>();
        if matching.len() != self.manufacturing.operations.len() {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let [stock, machining @ ..] = matching.as_slice() else {
            return Err(GeneralFabricationError::ExportBlocked);
        };
        if stock.kind != GeneralManufacturingKind::Stock
            || !stock.semantic_inputs.is_empty()
            || machining.is_empty()
            || machining.iter().any(|operation| {
                !matches!(
                    operation.kind,
                    GeneralManufacturingKind::CircularDrill | GeneralManufacturingKind::ProfileCut
                )
            })
            || options
                .vertical_pocket_tool_number
                .is_some_and(|tool| tool == 0 || tool > 999_999)
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let stock_frame =
            woodwop_stock_frame(&stock.machining).ok_or(GeneralFabricationError::ExportBlocked)?;
        let macros = machining
            .iter()
            .map(|operation| match operation.kind {
                GeneralManufacturingKind::CircularDrill => {
                    woodwop_drilling_macro(&operation.machining, stock_frame)
                }
                GeneralManufacturingKind::ProfileCut => {
                    options.vertical_pocket_tool_number.and_then(|tool| {
                        woodwop_vertical_pocket_macro(&operation.machining, stock_frame, tool)
                    })
                }
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(GeneralFabricationError::ExportBlocked)?;
        let [length_mm, width_mm, thickness_mm] = stock_frame.dimensions_mm;
        let length =
            format_btlx_positive_number(length_mm).ok_or(GeneralFabricationError::ExportBlocked)?;
        let width =
            format_btlx_positive_number(width_mm).ok_or(GeneralFabricationError::ExportBlocked)?;
        let thickness = format_btlx_positive_number(thickness_mm)
            .ok_or(GeneralFabricationError::ExportBlocked)?;
        let export_contract = if machining
            .iter()
            .any(|operation| operation.kind == GeneralManufacturingKind::ProfileCut)
        {
            WOODWOP_MPR_4_0_MACHINING_EXPORT_V2
        } else {
            WOODWOP_MPR_4_0_DRILL_EXPORT_V1
        };
        let mut output = format!(
            "[H\nVERSION=\"4.0\"\nOP=\"1\"\nINCH=\"0\"\nMAT=\"HOMAG\"\n_BSX={length}\n_BSY={width}\n_BSZ={thickness}\n\\{export_contract}\\\n<100 \\WerkStck\\\nLA=\"{length}\"\nBR=\"{width}\"\nDI=\"{thickness}\"\nFNX=\"0\"\nFNY=\"0\"\nAX=\"0\"\nAY=\"0\"\nRNX=\"0\"\nRNY=\"0\"\nRNZ=\"0\"\n"
        );
        for processing in macros {
            output.push_str(&processing);
        }
        output.push_str("!\n");
        if !output.is_ascii() {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        Ok(output.into_bytes())
    }

    pub fn woodwop_mpr_4_0_production_package(
        &self,
        snapshot: &Snapshot,
        options: WoodwopMprOptions,
    ) -> Result<Vec<HomagProductionProgram>, GeneralFabricationError> {
        self.woodwop_mpr_4_0_setup_a_programs(snapshot, options)?
            .into_iter()
            .map(|(instance_path, mpr)| {
                let program_name = homag_program_name(snapshot, &instance_path)?;
                let barcode_svg = homag_code128_svg(&program_name)
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                Ok(HomagProductionProgram {
                    schema: HOMAG_BHX_PRODUCTION_PACKAGE_V1,
                    program_name,
                    instance_path,
                    mpr,
                    barcode_svg,
                })
            })
            .collect()
    }

    /// Only the existing top-face/edge setup; no inferred workpiece flip or labels.
    /// Program identifiers belong to the destination adapter, not physical identity.
    pub fn woodwop_mpr_4_0_setup_a_programs(
        &self,
        snapshot: &Snapshot,
        options: WoodwopMprOptions,
    ) -> Result<Vec<(InstancePath, Vec<u8>)>, GeneralFabricationError> {
        self.bom_export(snapshot)?;
        self.manufacturing_export(snapshot)?;
        if options
            .vertical_pocket_tool_number
            .is_some_and(|tool| tool == 0 || tool > 999_999)
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let mut holes = Vec::new();
        for joint in snapshot.dowel_joints() {
            let projection = project_dowel_joint_contract(snapshot, joint)
                .map_err(|_| GeneralFabricationError::ExportBlocked)?;
            for pair in projection.pairs {
                holes.push(pair.first);
                holes.push(pair.second);
            }
        }
        let mut programs = Vec::new();
        let mut program_paths = BTreeSet::new();
        for row in self
            .bom
            .rows
            .iter()
            .filter(|row| row.item_kind != GeneralBomItemKind::Purchased)
        {
            let GeneralBodySource::Exact(row_source) = &row.source else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            let matching = self
                .manufacturing
                .operations
                .iter()
                .filter(|operation| {
                    operation.definition_id == row.definition_id && operation.source == *row_source
                })
                .collect::<Vec<_>>();
            let [stock, machining @ ..] = matching.as_slice() else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            if stock.kind != GeneralManufacturingKind::Stock
                || !stock.semantic_inputs.is_empty()
                || machining.iter().any(|operation| {
                    !matches!(
                        operation.kind,
                        GeneralManufacturingKind::CircularDrill
                            | GeneralManufacturingKind::ProfileCut
                    )
                })
            {
                return Err(GeneralFabricationError::ExportBlocked);
            }
            let stock_frame = woodwop_stock_frame(&stock.machining)
                .ok_or(GeneralFabricationError::ExportBlocked)?;
            for instance_path in &row.instances {
                let resolved = snapshot
                    .resolve_instance_path(instance_path)
                    .map_err(|_| GeneralFabricationError::ExportBlocked)?;
                if resolved.definition_id != row.definition_id
                    || !is_production_transform(resolved.world_transform)
                {
                    return Err(GeneralFabricationError::ExportBlocked);
                }
                let instance_holes = holes
                    .iter()
                    .filter(|hole| hole.instance_path == *instance_path)
                    .collect::<Vec<_>>();
                if machining.is_empty() && instance_holes.is_empty() {
                    continue;
                }
                let mut macros = machining
                    .iter()
                    .map(|operation| match operation.kind {
                        GeneralManufacturingKind::CircularDrill => {
                            woodwop_drilling_macro(&operation.machining, stock_frame)
                        }
                        GeneralManufacturingKind::ProfileCut => {
                            options.vertical_pocket_tool_number.and_then(|tool| {
                                woodwop_vertical_pocket_macro(
                                    &operation.machining,
                                    stock_frame,
                                    tool,
                                )
                            })
                        }
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                macros.extend(
                    instance_holes
                        .into_iter()
                        .map(|hole| woodwop_dowel_macro(hole, stock_frame))
                        .collect::<Option<Vec<_>>>()
                        .ok_or(GeneralFabricationError::ExportBlocked)?,
                );
                if !program_paths.insert(instance_path.clone()) {
                    return Err(GeneralFabricationError::ExportBlocked);
                }
                let mpr =
                    woodwop_mpr_output(stock_frame, &macros, HOMAG_BHX_PRODUCTION_PACKAGE_V1)?;
                programs.push((instance_path.clone(), mpr));
            }
        }
        if programs.is_empty() {
            return Err(GeneralFabricationError::NoSupportedGeometry);
        }
        Ok(programs)
    }

    pub fn btlx_2_3_1_export(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<u8>, GeneralFabricationError> {
        self.btlx_2_3_1_export_with_options(
            snapshot,
            BtlxExportOptions {
                profile_processing_request: BtlxProfileProcessingRequest::PortableFreeContour,
            },
        )
    }

    pub fn btlx_2_3_1_export_with_options(
        &self,
        snapshot: &Snapshot,
        options: BtlxExportOptions,
    ) -> Result<Vec<u8>, GeneralFabricationError> {
        self.bom_export(snapshot)?;
        self.manufacturing_export(snapshot)?;
        if !self
            .bom
            .rows
            .iter()
            .any(|row| row.item_kind == GeneralBomItemKind::Timber)
            || self.manufacturing.operations.is_empty()
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }

        let mut output = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<BTLx xmlns=\"https://www.design2machine.com\" Version=\"{BTLX_2_3_1_VERSION}\" Language=\"en\">\n  <Project Name=\"Ketchup\">\n    <Parts>\n"
        );
        let mut matched_operation_count = 0usize;
        let mut next_process_id = 1u32;
        let mut next_reference_plane_id = 100u32;
        for (index, row) in self
            .bom
            .rows
            .iter()
            .filter(|row| row.item_kind == GeneralBomItemKind::Timber)
            .enumerate()
        {
            let GeneralBodySource::Exact(row_source) = &row.source else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            let matching = self
                .manufacturing
                .operations
                .iter()
                .filter(|operation| {
                    operation.definition_id == row.definition_id && operation.source == *row_source
                })
                .collect::<Vec<_>>();
            let Some((stock, machining)) = matching.split_first() else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            matched_operation_count = matched_operation_count
                .checked_add(matching.len())
                .ok_or(GeneralFabricationError::ExportBlocked)?;
            if stock.kind != GeneralManufacturingKind::Stock
                || !stock.semantic_inputs.is_empty()
                || row.material_key != TIMBER_MATERIAL_V1
                || row.quantity == 0
                || row.quantity != row.instances.len()
            {
                return Err(GeneralFabricationError::ExportBlocked);
            }
            let Some((count, single_member_number)) =
                btlx_component_identifiers(row.quantity, index)
            else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            let Some((length_mm, width_mm, height_mm)) =
                rectangular_timber_stock_dimensions(&stock.machining)
            else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            let length = format_btlx_positive_number(length_mm)
                .ok_or(GeneralFabricationError::ExportBlocked)?;
            let width = format_btlx_positive_number(width_mm)
                .ok_or(GeneralFabricationError::ExportBlocked)?;
            let height = format_btlx_positive_number(height_mm)
                .ok_or(GeneralFabricationError::ExportBlocked)?;
            let processings = machining
                .iter()
                .map(|operation| btlx_processings(operation, options))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            output.push_str(&format!(
                "      <Part Count=\"{count}\" Length=\"{length}\" Width=\"{width}\" Height=\"{height}\" SingleMemberNumber=\"{single_member_number}\" Designation=\"definition-{}\" Material=\"{}\"{}\n",
                row.definition_id.0,
                TIMBER_MATERIAL_V1,
                if processings.is_empty() { "/>" } else { ">" }
            ));
            if processings.is_empty() {
                continue;
            }
            output.push_str("        <UserReferencePlanes>\n");
            let first_reference_plane_id = next_reference_plane_id;
            for processing in &processings {
                let reference_plane_id = next_reference_plane_id;
                next_reference_plane_id = next_reference_plane_id
                    .checked_add(1)
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                let (reference_point_mm, x_vector, y_vector) = processing.reference_plane();
                output.push_str(&format!(
                    "          <UserReferencePlane ID=\"{reference_plane_id}\">\n            <Position>\n              <ReferencePoint X=\"{}\" Y=\"{}\" Z=\"{}\"/>\n              <XVector X=\"{}\" Y=\"{}\" Z=\"{}\"/>\n              <YVector X=\"{}\" Y=\"{}\" Z=\"{}\"/>\n            </Position>\n          </UserReferencePlane>\n",
                    format_number(reference_point_mm[0]),
                    format_number(reference_point_mm[1]),
                    format_number(reference_point_mm[2]),
                    format_number(x_vector[0]),
                    format_number(x_vector[1]),
                    format_number(x_vector[2]),
                    format_number(y_vector[0]),
                    format_number(y_vector[1]),
                    format_number(y_vector[2])
                ));
            }
            output.push_str("        </UserReferencePlanes>\n        <Processings>\n");
            for (processing_index, processing) in processings.iter().enumerate() {
                let reference_plane_id = first_reference_plane_id
                    .checked_add(
                        u32::try_from(processing_index)
                            .map_err(|_| GeneralFabricationError::ExportBlocked)?,
                    )
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                let process_id = next_process_id;
                next_process_id = next_process_id
                    .checked_add(1)
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                output.push_str(&processing.xml(process_id, reference_plane_id));
            }
            output.push_str("        </Processings>\n      </Part>\n");
        }
        if matched_operation_count != self.manufacturing.operations.len() {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        output.push_str("    </Parts>\n  </Project>\n</BTLx>\n");
        Ok(output.into_bytes())
    }
}

fn btlx_component_identifiers(quantity: usize, zero_based_index: usize) -> Option<(i32, u32)> {
    let count = i32::try_from(quantity).ok().filter(|count| *count >= 1)?;
    let single_member_number = u32::try_from(zero_based_index.checked_add(1)?).ok()?;
    Some((count, single_member_number))
}

fn format_btlx_positive_number(value: f64) -> Option<String> {
    let formatted = format_number(value);
    formatted
        .parse::<f64>()
        .ok()
        .filter(|rounded| rounded.is_finite() && *rounded > 0.0)
        .map(|_| formatted)
}

#[derive(Clone, Debug, PartialEq)]
enum BtlxProcessing {
    Drilling(BtlxDrilling),
    FreeContour(BtlxFreeContour),
    SawContour(BtlxSawContour),
    MillContour(BtlxFreeContour),
}

impl BtlxProcessing {
    fn reference_plane(&self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        match self {
            Self::Drilling(drilling) => (
                drilling.reference_point_mm,
                drilling.x_vector,
                drilling.y_vector,
            ),
            Self::FreeContour(contour) | Self::MillContour(contour) => (
                contour.reference_point_mm,
                contour.x_vector,
                contour.y_vector,
            ),
            Self::SawContour(contour) => (
                contour.reference_point_mm,
                contour.x_vector,
                contour.y_vector,
            ),
        }
    }

    fn xml(&self, process_id: u32, reference_plane_id: u32) -> String {
        match self {
            Self::Drilling(drilling) => format!(
                "          <Drilling Name=\"Ketchup circular drilling\" ProcessID=\"{process_id}\" ReferencePlaneID=\"{reference_plane_id}\">\n            <StartX>{}</StartX>\n            <StartY>{}</StartY>\n            <Angle>0</Angle>\n            <Inclination>90</Inclination>\n            <DepthLimited>yes</DepthLimited>\n            <Depth>{}</Depth>\n            <Diameter>{}</Diameter>\n          </Drilling>\n",
                drilling.start_x, drilling.start_y, drilling.depth, drilling.diameter
            ),
            Self::FreeContour(contour) => {
                let mut xml = format!(
                    "          <FreeContour Name=\"Ketchup profile cut\" ProcessID=\"{process_id}\" ReferencePlaneID=\"{reference_plane_id}\" ToolPosition=\"{}\">\n            <Contour DepthBounded=\"yes\" Depth=\"{}\" Inclination=\"0\">\n              <StartPoint X=\"{}\" Y=\"{}\" Z=\"0\"/>\n",
                    contour.tool_position,
                    contour.depth,
                    contour.start_point[0],
                    contour.start_point[1]
                );
                for segment in &contour.segments {
                    xml.push_str(&segment.xml());
                }
                xml.push_str("            </Contour>\n          </FreeContour>\n");
                xml
            }
            Self::SawContour(contour) => format!(
                "          <SawContour Name=\"Ketchup groove edge pre-cut\" ProcessID=\"{process_id}\" ReferencePlaneID=\"{reference_plane_id}\" ToolPosition=\"{}\">\n            <Contour DepthBounded=\"yes\" Depth=\"{}\" Inclination=\"0\">\n              <StartPoint X=\"{}\" Y=\"{}\" Z=\"0\"/>\n              <Line><EndPoint X=\"{}\" Y=\"{}\" Z=\"0\"/></Line>\n            </Contour>\n          </SawContour>\n",
                contour.tool_position,
                contour.depth,
                contour.start_point[0],
                contour.start_point[1],
                contour.end_point[0],
                contour.end_point[1]
            ),
            Self::MillContour(contour) => {
                let mut xml = format!(
                    "          <MillContour Name=\"Ketchup groove interior milling\" ProcessID=\"{process_id}\" ReferencePlaneID=\"{reference_plane_id}\" ToolPosition=\"{}\">\n            <Contour DepthBounded=\"yes\" Depth=\"{}\" Inclination=\"0\">\n              <StartPoint X=\"{}\" Y=\"{}\" Z=\"0\"/>\n",
                    contour.tool_position,
                    contour.depth,
                    contour.start_point[0],
                    contour.start_point[1]
                );
                for segment in &contour.segments {
                    xml.push_str(&segment.xml());
                }
                xml.push_str("            </Contour>\n          </MillContour>\n");
                xml
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct BtlxDrilling {
    reference_point_mm: [f64; 3],
    x_vector: [f64; 3],
    y_vector: [f64; 3],
    start_x: String,
    start_y: String,
    depth: String,
    diameter: String,
}

#[derive(Clone, Debug, PartialEq)]
struct BtlxSawContour {
    reference_point_mm: [f64; 3],
    x_vector: [f64; 3],
    y_vector: [f64; 3],
    tool_position: &'static str,
    start_point: [String; 2],
    end_point: [String; 2],
    depth: String,
}

#[derive(Clone, Debug, PartialEq)]
enum BtlxContourSegment {
    Line {
        end_point: [String; 2],
    },
    Arc {
        end_point: [String; 2],
        point_on_arc: [String; 2],
    },
}

impl BtlxContourSegment {
    fn xml(&self) -> String {
        match self {
            Self::Line { end_point } => format!(
                "              <Line><EndPoint X=\"{}\" Y=\"{}\" Z=\"0\"/></Line>\n",
                end_point[0], end_point[1]
            ),
            Self::Arc {
                end_point,
                point_on_arc,
            } => format!(
                "              <Arc><EndPoint X=\"{}\" Y=\"{}\" Z=\"0\"/><PointOnArc X=\"{}\" Y=\"{}\" Z=\"0\"/></Arc>\n",
                end_point[0], end_point[1], point_on_arc[0], point_on_arc[1]
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct BtlxFreeContour {
    reference_point_mm: [f64; 3],
    x_vector: [f64; 3],
    y_vector: [f64; 3],
    tool_position: &'static str,
    start_point: [String; 2],
    segments: Vec<BtlxContourSegment>,
    depth: String,
}

fn btlx_processings(
    operation: &GeneralManufacturingOperation,
    options: BtlxExportOptions,
) -> Result<Vec<BtlxProcessing>, GeneralFabricationError> {
    match operation.kind {
        GeneralManufacturingKind::CircularDrill => btlx_drilling(&operation.machining)
            .map(|drilling| vec![BtlxProcessing::Drilling(drilling)])
            .ok_or(GeneralFabricationError::ExportBlocked),
        GeneralManufacturingKind::ThroughCut
        | GeneralManufacturingKind::ProfileCut
        | GeneralManufacturingKind::BooleanCut => {
            let contour = btlx_free_contour(&operation.machining)
                .ok_or(GeneralFabricationError::ExportBlocked)?;
            match options.profile_processing_request {
                BtlxProfileProcessingRequest::PortableFreeContour => {
                    Ok(vec![BtlxProcessing::FreeContour(contour)])
                }
                BtlxProfileProcessingRequest::EdgeSawCutsThenMillContour {
                    intermediate_saw_cuts,
                } => {
                    let saw_contours = btlx_edge_saw_contours(
                        &operation.machining,
                        &contour,
                        intermediate_saw_cuts,
                    )
                    .ok_or(GeneralFabricationError::BtlxProfileRequestUnsupported)?;
                    let mut processings = saw_contours
                        .into_iter()
                        .map(BtlxProcessing::SawContour)
                        .collect::<Vec<_>>();
                    processings.push(BtlxProcessing::MillContour(contour));
                    Ok(processings)
                }
            }
        }
        _ => Err(GeneralFabricationError::ExportBlocked),
    }
}

fn btlx_edge_saw_contours(
    geometry: &GeneralMachiningGeometry,
    contour: &BtlxFreeContour,
    intermediate_saw_cuts: u8,
) -> Option<Vec<BtlxSawContour>> {
    if intermediate_saw_cuts > 32 {
        return None;
    }
    let GeneralMachiningGeometry::ProfileCut { segments, .. } = geometry else {
        return None;
    };
    let edges = segments
        .iter()
        .map(|segment| {
            let GeneralMachiningSegment::Line { start_mm, end_mm } = segment else {
                return None;
            };
            Some((*start_mm, *end_mm))
        })
        .collect::<Option<Vec<_>>>()?;
    let [first, second, third, fourth] = edges.as_slice() else {
        return None;
    };
    let vector = |(start, end): &([f64; 2], [f64; 2])| [end[0] - start[0], end[1] - start[1]];
    let dot = |left: [f64; 2], right: [f64; 2]| left[0] * right[0] + left[1] * right[1];
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let vectors = [vector(first), vector(second), vector(third), vector(fourth)];
    let close = |left: f64, right: f64| (left - right).abs() <= 1.0e-9;
    if !close(dot(vectors[0], vectors[1]), 0.0)
        || !close(dot(vectors[1], vectors[2]), 0.0)
        || !close(cross(vectors[0], vectors[2]), 0.0)
        || !close(cross(vectors[1], vectors[3]), 0.0)
        || !close(dot(vectors[0], vectors[0]), dot(vectors[2], vectors[2]))
        || !close(dot(vectors[1], vectors[1]), dot(vectors[3], vectors[3]))
    {
        return None;
    }
    let [first_boundary, second_boundary] =
        if dot(vectors[0], vectors[0]) >= dot(vectors[1], vectors[1]) {
            [*first, *third]
        } else {
            [*second, *fourth]
        };
    let saw_contour = |(start, end): ([f64; 2], [f64; 2]), tool_position| BtlxSawContour {
        reference_point_mm: contour.reference_point_mm,
        x_vector: contour.x_vector,
        y_vector: contour.y_vector,
        tool_position,
        start_point: start.map(format_number),
        end_point: end.map(format_number),
        depth: contour.depth.clone(),
    };
    let mut result = Vec::with_capacity(usize::from(intermediate_saw_cuts) + 2);
    result.push(saw_contour(first_boundary, contour.tool_position));
    for index in 1..=intermediate_saw_cuts {
        let fraction = f64::from(index) / (f64::from(intermediate_saw_cuts) + 1.0);
        let interpolate = |start: [f64; 2], end: [f64; 2]| {
            std::array::from_fn(|axis| start[axis] + (end[axis] - start[axis]) * fraction)
        };
        result.push(saw_contour(
            (
                interpolate(first_boundary.0, second_boundary.1),
                interpolate(first_boundary.1, second_boundary.0),
            ),
            "center",
        ));
    }
    result.push(saw_contour(second_boundary, contour.tool_position));
    Some(result)
}

fn btlx_drilling(geometry: &GeneralMachiningGeometry) -> Option<BtlxDrilling> {
    let GeneralMachiningGeometry::CircularDrill {
        frame,
        center_mm,
        diameter_mm,
        start_mm,
        end_mm,
    } = geometry
    else {
        return None;
    };
    if !machining_frame_is_right_handed(frame)
        || center_mm.iter().any(|value| !value.is_finite())
        || !start_mm.is_finite()
        || !end_mm.is_finite()
        || end_mm <= start_mm
    {
        return None;
    }
    let depth_mm = end_mm - start_mm;
    let reference_point_mm = btlx_part_coordinate(std::array::from_fn(|axis| {
        frame.origin_mm[axis] + frame.normal[axis] * start_mm
    }));
    Some(BtlxDrilling {
        reference_point_mm,
        x_vector: btlx_part_coordinate(frame.x_axis),
        y_vector: btlx_part_coordinate(frame.y_axis),
        start_x: format_btlx_number_in_range(center_mm[0], -100_000.0, 100_000.0)?,
        start_y: format_btlx_number_in_range(center_mm[1], -50_000.0, 50_000.0)?,
        depth: format_btlx_number_in_range(depth_mm, f64::MIN_POSITIVE, 50_000.0)?,
        diameter: format_btlx_number_in_range(*diameter_mm, f64::MIN_POSITIVE, 50_000.0)?,
    })
}

fn btlx_arc_points(
    start: [f64; 2],
    end: [f64; 2],
    center: [f64; 2],
    clockwise: bool,
) -> Option<Vec<[f64; 2]>> {
    if start
        .into_iter()
        .chain(end)
        .chain(center)
        .any(|coordinate| !coordinate.is_finite())
        || start == end
    {
        return None;
    }
    let start_vector = [start[0] - center[0], start[1] - center[1]];
    let end_vector = [end[0] - center[0], end[1] - center[1]];
    let start_radius = start_vector[0].hypot(start_vector[1]);
    let end_radius = end_vector[0].hypot(end_vector[1]);
    let radius_tolerance = 1.0e-9 * start_radius.max(end_radius).max(1.0);
    if start_radius <= 0.0 || (start_radius - end_radius).abs() > radius_tolerance {
        return None;
    }
    let start_angle = start_vector[1].atan2(start_vector[0]);
    let mut sweep = end_vector[1].atan2(end_vector[0]) - start_angle;
    if clockwise {
        if sweep >= 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else if sweep <= 0.0 {
        sweep += std::f64::consts::TAU;
    }
    let mut subdivisions = (sweep.abs() / (std::f64::consts::PI / 18.0))
        .ceil()
        .max(2.0) as usize;
    subdivisions += subdivisions % 2;
    Some(
        (1..=subdivisions)
            .map(|index| {
                let angle = start_angle + sweep * index as f64 / subdivisions as f64;
                if index == subdivisions {
                    end
                } else {
                    [
                        center[0] + start_radius * angle.cos(),
                        center[1] + start_radius * angle.sin(),
                    ]
                }
            })
            .collect(),
    )
}

fn btlx_free_contour(geometry: &GeneralMachiningGeometry) -> Option<BtlxFreeContour> {
    let GeneralMachiningGeometry::ProfileCut {
        frame,
        segments,
        start_mm,
        end_mm,
    } = geometry
    else {
        return None;
    };
    if !machining_frame_is_right_handed(frame)
        || !start_mm.is_finite()
        || !end_mm.is_finite()
        || end_mm <= start_mm
        || segments.len() < 2
    {
        return None;
    }
    let endpoints = segments
        .iter()
        .map(|segment| match segment {
            GeneralMachiningSegment::Line { start_mm, end_mm } => (start_mm
                .iter()
                .chain(end_mm)
                .all(|coordinate| coordinate.is_finite())
                && start_mm != end_mm)
                .then_some((*start_mm, *end_mm)),
            GeneralMachiningSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => btlx_arc_points(*start_mm, *end_mm, *center_mm, *clockwise)
                .map(|_| (*start_mm, *end_mm)),
        })
        .collect::<Option<Vec<_>>>()?;
    if !endpoints
        .iter()
        .zip(endpoints.iter().cycle().skip(1))
        .all(|((_, end), (next_start, _))| end == next_start)
    {
        return None;
    }
    let start_point = endpoints[0].0.map(format_number);
    let parse_point =
        |point: &[String; 2]| Some([point[0].parse::<f64>().ok()?, point[1].parse::<f64>().ok()?]);
    let mut rounded_points = vec![parse_point(&start_point)?];
    let mut btlx_segments = Vec::with_capacity(segments.len());
    for segment in segments {
        match segment {
            GeneralMachiningSegment::Line { end_mm, .. } => {
                let end_point = end_mm.map(format_number);
                rounded_points.push(parse_point(&end_point)?);
                btlx_segments.push(BtlxContourSegment::Line { end_point });
            }
            GeneralMachiningSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let arc_points = btlx_arc_points(*start_mm, *end_mm, *center_mm, *clockwise)?;
                let point_on_arc = arc_points[arc_points.len() / 2 - 1].map(format_number);
                let end_point = end_mm.map(format_number);
                if point_on_arc == start_mm.map(format_number) || point_on_arc == end_point {
                    return None;
                }
                rounded_points.extend(
                    arc_points
                        .iter()
                        .map(|point| parse_point(&point.map(format_number)))
                        .collect::<Option<Vec<_>>>()?,
                );
                btlx_segments.push(BtlxContourSegment::Arc {
                    end_point,
                    point_on_arc,
                });
            }
        }
    }
    let rounded_edges = rounded_points
        .windows(2)
        .map(|points| (points[0], points[1]))
        .collect::<Vec<_>>();
    let signed_double_area = rounded_edges
        .iter()
        .map(|(start, end)| start[0] * end[1] - end[0] * start[1])
        .sum::<f64>();
    if rounded_edges.iter().any(|(start, end)| start == end)
        || rounded_points.first() != rounded_points.last()
        || !btlx_polygon_is_simple(&rounded_edges)
        || !signed_double_area.is_finite()
        || signed_double_area == 0.0
    {
        return None;
    }
    let depth_mm = end_mm - start_mm;
    Some(BtlxFreeContour {
        reference_point_mm: btlx_part_coordinate(std::array::from_fn(|axis| {
            frame.origin_mm[axis] + frame.normal[axis] * start_mm
        })),
        x_vector: btlx_part_coordinate(frame.x_axis),
        y_vector: btlx_part_coordinate(frame.y_axis),
        tool_position: if signed_double_area > 0.0 {
            "left"
        } else {
            "right"
        },
        start_point,
        segments: btlx_segments,
        depth: format_btlx_number_in_range(depth_mm, f64::MIN_POSITIVE, 100_000.0)?,
    })
}

fn btlx_polygon_is_simple(edges: &[([f64; 2], [f64; 2])]) -> bool {
    let cross = |origin: [f64; 2], first: [f64; 2], second: [f64; 2]| {
        (first[0] - origin[0]) * (second[1] - origin[1])
            - (first[1] - origin[1]) * (second[0] - origin[0])
    };
    let point_on_segment = |point: [f64; 2], start: [f64; 2], end: [f64; 2]| {
        cross(start, end, point) == 0.0
            && point[0] >= start[0].min(end[0])
            && point[0] <= start[0].max(end[0])
            && point[1] >= start[1].min(end[1])
            && point[1] <= start[1].max(end[1])
    };
    let intersects = |(left_start, left_end): ([f64; 2], [f64; 2]),
                      (right_start, right_end): ([f64; 2], [f64; 2])| {
        let left_start_side = cross(left_start, left_end, right_start);
        let left_end_side = cross(left_start, left_end, right_end);
        let right_start_side = cross(right_start, right_end, left_start);
        let right_end_side = cross(right_start, right_end, left_end);
        (left_start_side * left_end_side < 0.0 && right_start_side * right_end_side < 0.0)
            || point_on_segment(right_start, left_start, left_end)
            || point_on_segment(right_end, left_start, left_end)
            || point_on_segment(left_start, right_start, right_end)
            || point_on_segment(left_end, right_start, right_end)
    };

    let edge_count = edges.len();
    if (0..edge_count).any(|index| {
        let previous = edges[(index + edge_count - 1) % edge_count].0;
        let current = edges[index].0;
        let next = edges[(index + 1) % edge_count].0;
        cross(previous, current, next) == 0.0
    }) {
        return false;
    }
    for left in 0..edge_count {
        for right in (left + 1)..edge_count {
            if right == left + 1 || (left == 0 && right == edge_count - 1) {
                continue;
            }
            if intersects(edges[left], edges[right]) {
                return false;
            }
        }
    }
    true
}

fn format_btlx_number_in_range(value: f64, minimum: f64, maximum: f64) -> Option<String> {
    let formatted = format_number(value);
    formatted
        .parse::<f64>()
        .ok()
        .filter(|rounded| rounded.is_finite() && *rounded >= minimum && *rounded <= maximum)
        .map(|_| formatted)
}

fn btlx_part_coordinate(definition_coordinate: [f64; 3]) -> [f64; 3] {
    [
        definition_coordinate[2],
        definition_coordinate[0],
        definition_coordinate[1],
    ]
}

fn machining_frame_is_right_handed(frame: &GeneralMachiningFrame) -> bool {
    if frame
        .origin_mm
        .iter()
        .chain(&frame.x_axis)
        .chain(&frame.y_axis)
        .chain(&frame.normal)
        .any(|value| !value.is_finite())
    {
        return false;
    }
    let dot = |left: [f64; 3], right: [f64; 3]| {
        left.into_iter()
            .zip(right)
            .map(|(left, right)| left * right)
            .sum::<f64>()
    };
    let cross = [
        frame.x_axis[1] * frame.y_axis[2] - frame.x_axis[2] * frame.y_axis[1],
        frame.x_axis[2] * frame.y_axis[0] - frame.x_axis[0] * frame.y_axis[2],
        frame.x_axis[0] * frame.y_axis[1] - frame.x_axis[1] * frame.y_axis[0],
    ];
    let close = |left: f64, right: f64| (left - right).abs() <= 1.0e-12;
    close(dot(frame.x_axis, frame.x_axis), 1.0)
        && close(dot(frame.y_axis, frame.y_axis), 1.0)
        && close(dot(frame.normal, frame.normal), 1.0)
        && close(dot(frame.x_axis, frame.y_axis), 0.0)
        && cross
            .into_iter()
            .zip(frame.normal)
            .all(|(actual, expected)| close(actual, expected))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WoodwopStockFrame {
    definition_axes: [usize; 3],
    dimensions_mm: [f64; 3],
}

fn woodwop_stock_frame(geometry: &GeneralMachiningGeometry) -> Option<WoodwopStockFrame> {
    let (definition_z_mm, definition_x_mm, definition_y_mm) =
        rectangular_timber_stock_dimensions(geometry)?;
    let definition_dimensions = [definition_x_mm, definition_y_mm, definition_z_mm];
    let mut definition_axes = [0, 1, 2];
    definition_axes.sort_by(|left, right| {
        definition_dimensions[*right]
            .total_cmp(&definition_dimensions[*left])
            .then(left.cmp(right))
    });
    Some(WoodwopStockFrame {
        definition_axes,
        dimensions_mm: definition_axes.map(|axis| definition_dimensions[axis]),
    })
}

fn woodwop_coordinate(definition_coordinate: [f64; 3], stock_frame: WoodwopStockFrame) -> [f64; 3] {
    stock_frame
        .definition_axes
        .map(|axis| definition_coordinate[axis])
}

fn homag_program_name(
    snapshot: &Snapshot,
    instance_path: &InstancePath,
) -> Result<String, GeneralFabricationError> {
    snapshot
        .production_code(instance_path)
        .filter(|code| code.len() == 12)
        .map(str::to_owned)
        .ok_or(GeneralFabricationError::ExportBlocked)
}

fn woodwop_mpr_output(
    stock_frame: WoodwopStockFrame,
    macros: &[String],
    export_contract: &str,
) -> Result<Vec<u8>, GeneralFabricationError> {
    let [length_mm, width_mm, thickness_mm] = stock_frame.dimensions_mm;
    let length =
        format_btlx_positive_number(length_mm).ok_or(GeneralFabricationError::ExportBlocked)?;
    let width =
        format_btlx_positive_number(width_mm).ok_or(GeneralFabricationError::ExportBlocked)?;
    let thickness =
        format_btlx_positive_number(thickness_mm).ok_or(GeneralFabricationError::ExportBlocked)?;
    let mut output = format!(
        "[H\nVERSION=\"4.0\"\nOP=\"1\"\nINCH=\"0\"\nMAT=\"HOMAG\"\n_BSX={length}\n_BSY={width}\n_BSZ={thickness}\n\\{export_contract}\\\n<100 \\WerkStck\\\nLA=\"{length}\"\nBR=\"{width}\"\nDI=\"{thickness}\"\nFNX=\"0\"\nFNY=\"0\"\nAX=\"0\"\nAY=\"0\"\nRNX=\"0\"\nRNY=\"0\"\nRNZ=\"0\"\n"
    );
    for processing in macros {
        output.push_str(processing);
    }
    output.push_str("!\n");
    if !output.is_ascii() {
        return Err(GeneralFabricationError::ExportBlocked);
    }
    Ok(output.into_bytes())
}

fn woodwop_dowel_macro(hole: &DowelHole, stock_frame: WoodwopStockFrame) -> Option<String> {
    let normal = hole.inward_unit_local;
    let tolerance = 1.0e-12;
    let (x_axis, y_axis) = if normal[0] >= 1.0 - tolerance
        && normal[1].abs() <= tolerance
        && normal[2].abs() <= tolerance
    {
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0])
    } else if normal[0] <= -1.0 + tolerance
        && normal[1].abs() <= tolerance
        && normal[2].abs() <= tolerance
    {
        ([0.0, 1.0, 0.0], [0.0, 0.0, -1.0])
    } else if normal[1] >= 1.0 - tolerance
        && normal[0].abs() <= tolerance
        && normal[2].abs() <= tolerance
    {
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0])
    } else if normal[1] <= -1.0 + tolerance
        && normal[0].abs() <= tolerance
        && normal[2].abs() <= tolerance
    {
        ([0.0, 0.0, 1.0], [-1.0, 0.0, 0.0])
    } else if normal[2] >= 1.0 - tolerance
        && normal[0].abs() <= tolerance
        && normal[1].abs() <= tolerance
    {
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
    } else if normal[2] <= -1.0 + tolerance
        && normal[0].abs() <= tolerance
        && normal[1].abs() <= tolerance
    {
        ([1.0, 0.0, 0.0], [0.0, -1.0, 0.0])
    } else {
        return None;
    };
    woodwop_drilling_macro(
        &GeneralMachiningGeometry::CircularDrill {
            frame: GeneralMachiningFrame {
                origin_mm: hole.entry_local_mm,
                x_axis,
                y_axis,
                normal,
            },
            center_mm: [0.0, 0.0],
            diameter_mm: hole.diameter_mm,
            start_mm: 0.0,
            end_mm: hole.depth_mm,
        },
        stock_frame,
    )
}

fn homag_code128_svg(program_name: &str) -> Option<Vec<u8>> {
    if program_name.len() != 12
        || !program_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return None;
    }
    const PATTERNS: [&str; 107] = [
        "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
        "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
        "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
        "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
        "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
        "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
        "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
        "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
        "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
        "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
        "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
        "113141", "114131", "311141", "411131", "211412", "211214", "211232", "2331112",
    ];
    let values = program_name
        .bytes()
        .map(|byte| usize::from(byte - 32))
        .collect::<Vec<_>>();
    let checksum = (104
        + values
            .iter()
            .enumerate()
            .map(|(index, value)| (index + 1) * value)
            .sum::<usize>())
        % 103;
    let symbols = std::iter::once(104)
        .chain(values)
        .chain([checksum, 106])
        .collect::<Vec<_>>();
    let module_count = 20
        + symbols
            .iter()
            .map(|symbol| {
                PATTERNS[*symbol]
                    .bytes()
                    .map(|digit| usize::from(digit - b'0'))
                    .sum::<usize>()
            })
            .sum::<usize>();
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" data-schema=\"{HOMAG_CODE128_LABEL_V1}\" viewBox=\"0 0 {module_count} 80\" role=\"img\" aria-label=\"HOMAG program {program_name}\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/>"
    );
    let mut x = 10usize;
    for symbol in symbols {
        let mut bar = true;
        for width in PATTERNS[symbol]
            .bytes()
            .map(|digit| usize::from(digit - b'0'))
        {
            if bar {
                svg.push_str(&format!(
                    "<rect x=\"{x}\" y=\"4\" width=\"{width}\" height=\"56\" fill=\"black\"/>"
                ));
            }
            x += width;
            bar = !bar;
        }
    }
    svg.push_str(&format!(
        "<text x=\"{}\" y=\"74\" text-anchor=\"middle\" font-family=\"monospace\" font-size=\"9\">{program_name}</text></svg>\n",
        module_count / 2
    ));
    Some(svg.into_bytes())
}

fn woodwop_drilling_macro(
    geometry: &GeneralMachiningGeometry,
    stock_frame: WoodwopStockFrame,
) -> Option<String> {
    let GeneralMachiningGeometry::CircularDrill {
        frame,
        center_mm,
        diameter_mm,
        start_mm,
        end_mm,
    } = geometry
    else {
        return None;
    };
    if !machining_frame_is_right_handed(frame)
        || center_mm.iter().any(|value| !value.is_finite())
        || !diameter_mm.is_finite()
        || *diameter_mm <= 0.0
        || !start_mm.is_finite()
        || !end_mm.is_finite()
        || end_mm <= start_mm
    {
        return None;
    }
    let definition_entry = std::array::from_fn(|axis| {
        frame.origin_mm[axis]
            + frame.x_axis[axis] * center_mm[0]
            + frame.y_axis[axis] * center_mm[1]
            + frame.normal[axis] * start_mm
    });
    let entry = woodwop_coordinate(definition_entry, stock_frame);
    let direction = woodwop_coordinate(frame.normal, stock_frame);
    let [length_mm, width_mm, thickness_mm] = stock_frame.dimensions_mm;
    let dimensions = stock_frame.dimensions_mm;
    let tolerance = dimensions.into_iter().fold(1.0_f64, f64::max) * 1.0e-9;
    let axis = (0..3).find(|candidate| {
        direction[*candidate].abs() >= 1.0 - 1.0e-12
            && direction
                .iter()
                .enumerate()
                .all(|(index, value)| index == *candidate || value.abs() <= 1.0e-12)
    })?;
    let sign = if direction[axis] > 0.0 { 1.0 } else { -1.0 };
    let expected_entry = if sign > 0.0 { 0.0 } else { dimensions[axis] };
    let depth_mm = end_mm - start_mm;
    if (entry[axis] - expected_entry).abs() > tolerance
        || depth_mm >= dimensions[axis] - tolerance
        || entry.iter().enumerate().any(|(index, value)| {
            !value.is_finite() || *value < -tolerance || *value > dimensions[index] + tolerance
        })
    {
        return None;
    }
    let coordinate = |value: f64, maximum: f64| {
        format_btlx_number_in_range(value.clamp(0.0, maximum), 0.0, maximum)
    };
    let x = coordinate(entry[0], length_mm)?;
    let y = coordinate(entry[1], width_mm)?;
    let z = coordinate(entry[2], thickness_mm)?;
    let depth = format_btlx_number_in_range(depth_mm, f64::MIN_POSITIVE, dimensions[axis])?;
    let diameter = format_btlx_number_in_range(*diameter_mm, f64::MIN_POSITIVE, 50_000.0)?;
    if axis == 2 {
        if sign > 0.0 {
            return None;
        }
        Some(format!(
            "<102 \\BohrVert\\\nXA=\"{x}\"\nYA=\"{y}\"\nBM=\"LS\"\nTI=\"{depth}\"\nDU=\"{diameter}\"\nAN=\"1\"\nMI=\"0\"\nAB=\"0\"\nF_=\"STANDARD\"\nS_=\"1\"\nKO=\"0\"\n??=\"1\"\nEN=\"1\"\n"
        ))
    } else {
        let mode = match (axis, sign > 0.0) {
            (0, true) => "XP",
            (0, false) => "XM",
            (1, true) => "YP",
            (1, false) => "YM",
            _ => return None,
        };
        Some(format!(
            "<103 \\BohrHoriz\\\nXA=\"{x}\"\nYA=\"{y}\"\nZA=\"{z}\"\nBM=\"{mode}\"\nTI=\"{depth}\"\nDU=\"{diameter}\"\nAN=\"1\"\nMI=\"0\"\nAB=\"0\"\nF_=\"STANDARD\"\nKO=\"0\"\n??=\"1\"\nEN=\"1\"\n"
        ))
    }
}

fn woodwop_vertical_pocket_macro(
    geometry: &GeneralMachiningGeometry,
    stock_frame: WoodwopStockFrame,
    tool_number: u32,
) -> Option<String> {
    let GeneralMachiningGeometry::ProfileCut {
        frame,
        segments,
        start_mm,
        end_mm,
    } = geometry
    else {
        return None;
    };
    if tool_number == 0
        || tool_number > 999_999
        || !machining_frame_is_right_handed(frame)
        || !start_mm.is_finite()
        || !end_mm.is_finite()
        || end_mm <= start_mm
        || segments.len() != 4
    {
        return None;
    }
    let direction = woodwop_coordinate(frame.normal, stock_frame);
    if direction[2] >= -1.0 + 1.0e-12
        || direction[0].abs() > 1.0e-12
        || direction[1].abs() > 1.0e-12
    {
        return None;
    }
    let dimensions = stock_frame.dimensions_mm;
    let tolerance = dimensions.into_iter().fold(1.0_f64, f64::max) * 1.0e-9;
    let depth_mm = end_mm - start_mm;
    if depth_mm >= dimensions[2] - tolerance {
        return None;
    }

    let profile_point = |point: [f64; 2]| {
        woodwop_coordinate(
            std::array::from_fn(|axis| {
                frame.origin_mm[axis]
                    + frame.x_axis[axis] * point[0]
                    + frame.y_axis[axis] * point[1]
                    + frame.normal[axis] * start_mm
            }),
            stock_frame,
        )
    };
    let mut vertices = Vec::with_capacity(4);
    let mut previous_end = None;
    for segment in segments {
        let GeneralMachiningSegment::Line { start_mm, end_mm } = segment else {
            return None;
        };
        let start = profile_point(*start_mm);
        let end = profile_point(*end_mm);
        if start.into_iter().chain(end).any(|value| !value.is_finite())
            || (start[2] - dimensions[2]).abs() > tolerance
            || (end[2] - dimensions[2]).abs() > tolerance
            || previous_end.is_some_and(|previous: [f64; 3]| {
                (0..3).any(|axis| (previous[axis] - start[axis]).abs() > tolerance)
            })
        {
            return None;
        }
        let horizontal = (start[1] - end[1]).abs() <= tolerance;
        let vertical = (start[0] - end[0]).abs() <= tolerance;
        if horizontal == vertical {
            return None;
        }
        vertices.push(start);
        previous_end = Some(end);
    }
    let final_end = previous_end?;
    if (0..3).any(|axis| (final_end[axis] - vertices[0][axis]).abs() > tolerance) {
        return None;
    }
    let minimum = [
        vertices
            .iter()
            .map(|point| point[0])
            .fold(f64::INFINITY, f64::min),
        vertices
            .iter()
            .map(|point| point[1])
            .fold(f64::INFINITY, f64::min),
    ];
    let maximum = [
        vertices
            .iter()
            .map(|point| point[0])
            .fold(f64::NEG_INFINITY, f64::max),
        vertices
            .iter()
            .map(|point| point[1])
            .fold(f64::NEG_INFINITY, f64::max),
    ];
    if minimum[0] < -tolerance
        || minimum[1] < -tolerance
        || maximum[0] > dimensions[0] + tolerance
        || maximum[1] > dimensions[1] + tolerance
        || maximum[0] - minimum[0] <= tolerance
        || maximum[1] - minimum[1] <= tolerance
        || vertices.iter().any(|point| {
            ![minimum[0], maximum[0]]
                .into_iter()
                .any(|value| (point[0] - value).abs() <= tolerance)
                || ![minimum[1], maximum[1]]
                    .into_iter()
                    .any(|value| (point[1] - value).abs() <= tolerance)
        })
    {
        return None;
    }
    let x = format_btlx_number_in_range((minimum[0] + maximum[0]) / 2.0, 0.0, dimensions[0])?;
    let y = format_btlx_number_in_range((minimum[1] + maximum[1]) / 2.0, 0.0, dimensions[1])?;
    let length = format_btlx_positive_number(maximum[0] - minimum[0])?;
    let width = format_btlx_positive_number(maximum[1] - minimum[1])?;
    let depth = format_btlx_number_in_range(depth_mm, f64::MIN_POSITIVE, dimensions[2])?;
    Some(format!(
        "<112 \\Tasche\\\nXA=\"{x}\"\nYA=\"{y}\"\nLA=\"{length}\"\nBR=\"{width}\"\nRD=\"0\"\nWI=\"0\"\nTI=\"{depth}\"\nZT=\"0\"\nXY=\"80\"\nDS=\"1\"\nT_=\"{tool_number}\"\nF_=\"STANDARD\"\nKO=\"0\"\n??=\"1\"\nEN=\"1\"\n"
    ))
}

fn rectangular_timber_stock_dimensions(
    geometry: &GeneralMachiningGeometry,
) -> Option<(f64, f64, f64)> {
    let GeneralMachiningGeometry::TimberStock {
        frame,
        cross_section,
        start_mm,
        length_axis,
        length_mm,
        cross_section_width_mm,
        cross_section_height_mm,
    } = geometry
    else {
        return None;
    };
    if *frame != identity_machining_frame()
        || *start_mm != [0.0, 0.0, 0.0]
        || *length_axis != [0.0, 0.0, 1.0]
    {
        return None;
    }
    rectangular_stock_profile_dimensions(
        cross_section,
        *length_mm,
        *cross_section_width_mm,
        *cross_section_height_mm,
    )
}

fn rectangular_stock_profile_dimensions(
    cross_section: &[GeneralMachiningSegment],
    length_mm: f64,
    cross_section_width_mm: f64,
    cross_section_height_mm: f64,
) -> Option<(f64, f64, f64)> {
    if cross_section.len() != 4
        || [length_mm, cross_section_width_mm, cross_section_height_mm]
            .into_iter()
            .any(|value| !value.is_finite() || value <= 0.0)
    {
        return None;
    }

    let mut edges = Vec::with_capacity(4);
    for segment in cross_section {
        let GeneralMachiningSegment::Line { start_mm, end_mm } = segment else {
            return None;
        };
        if start_mm
            .iter()
            .chain(end_mm)
            .any(|coordinate| !coordinate.is_finite())
            || (start_mm[0] == end_mm[0]) == (start_mm[1] == end_mm[1])
        {
            return None;
        }
        edges.push((*start_mm, *end_mm));
    }
    if !edges
        .iter()
        .zip(edges.iter().cycle().skip(1))
        .all(|((_, end), (next_start, _))| end == next_start)
    {
        return None;
    }

    let minimum = [
        edges
            .iter()
            .flat_map(|(start, end)| [start[0], end[0]])
            .fold(f64::INFINITY, f64::min),
        edges
            .iter()
            .flat_map(|(start, end)| [start[1], end[1]])
            .fold(f64::INFINITY, f64::min),
    ];
    let maximum = [
        edges
            .iter()
            .flat_map(|(start, end)| [start[0], end[0]])
            .fold(f64::NEG_INFINITY, f64::max),
        edges
            .iter()
            .flat_map(|(start, end)| [start[1], end[1]])
            .fold(f64::NEG_INFINITY, f64::max),
    ];
    let expected_edges = [
        ([minimum[0], minimum[1]], [maximum[0], minimum[1]]),
        ([maximum[0], minimum[1]], [maximum[0], maximum[1]]),
        ([maximum[0], maximum[1]], [minimum[0], maximum[1]]),
        ([minimum[0], maximum[1]], [minimum[0], minimum[1]]),
    ];
    if expected_edges.iter().any(|expected| {
        edges
            .iter()
            .filter(|actual| {
                **actual == *expected || (actual.0 == expected.1 && actual.1 == expected.0)
            })
            .count()
            != 1
    }) {
        return None;
    }
    let width_mm = maximum[0] - minimum[0];
    let height_mm = maximum[1] - minimum[1];
    (width_mm == cross_section_width_mm && height_mm == cross_section_height_mm)
        .then_some((length_mm, width_mm, height_mm))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BomOccurrenceMetadata {
    item_kind: GeneralBomItemKind,
    material_key: String,
}

fn bom_occurrence_metadata(
    snapshot: &Snapshot,
) -> Result<BTreeMap<OccurrenceId, BomOccurrenceMetadata>, GeneralFabricationError> {
    let role_dimensions = snapshot
        .classification_dimensions()
        .filter(|dimension| dimension.name() == FABRICATION_ROLE_DIMENSION_V1)
        .collect::<Vec<_>>();
    let [role_dimension] = role_dimensions.as_slice() else {
        return Err(if role_dimensions.is_empty() {
            GeneralFabricationError::FabricationRoleDimensionMissing
        } else {
            GeneralFabricationError::FabricationRoleDimensionAmbiguous
        });
    };
    let material_dimensions = snapshot
        .classification_dimensions()
        .filter(|dimension| dimension.name() == MATERIAL_DIMENSION_V1)
        .collect::<Vec<_>>();
    let material_dimension = match material_dimensions.as_slice() {
        [] => None,
        [dimension] => Some(*dimension),
        _ => return Err(GeneralFabricationError::MaterialDimensionAmbiguous),
    };

    let mut metadata = BTreeMap::new();
    for occurrence in snapshot.occurrences() {
        let Some(role_category_id) =
            snapshot.occurrence_classification(occurrence.id(), role_dimension.id())
        else {
            continue;
        };
        let Some(role_category) = role_dimension.category(role_category_id) else {
            return Err(GeneralFabricationError::InvalidBomMetadata);
        };
        let item_kind = match role_category.name() {
            TIMBER_MEMBER_ROLE_V1 => GeneralBomItemKind::Timber,
            MANUFACTURED_ITEM_ROLE_V1 => GeneralBomItemKind::Manufactured,
            PURCHASED_ITEM_ROLE_V1 => GeneralBomItemKind::Purchased,
            _ => continue,
        };
        let material_key = if item_kind == GeneralBomItemKind::Timber {
            TIMBER_MATERIAL_V1.to_owned()
        } else if let Some(material_dimension) = material_dimension {
            snapshot
                .occurrence_classification(occurrence.id(), material_dimension.id())
                .map(|category_id| {
                    material_dimension
                        .category(category_id)
                        .map(|category| category.name().to_owned())
                        .ok_or(GeneralFabricationError::InvalidBomMetadata)
                })
                .transpose()?
                .unwrap_or_else(|| UNSPECIFIED_MATERIAL_V1.to_owned())
        } else {
            UNSPECIFIED_MATERIAL_V1.to_owned()
        };
        if !manufacturing_export_token_is_safe(&material_key) {
            return Err(GeneralFabricationError::InvalidBomMetadata);
        }
        metadata.insert(
            occurrence.id(),
            BomOccurrenceMetadata {
                item_kind,
                material_key,
            },
        );
    }
    Ok(metadata)
}

fn straight_weldment_member(
    snapshot: &Snapshot,
    member_id: FeatureId,
) -> Result<(FeatureId, f64, [f64; 3], [f64; 3]), GeneralFabricationError> {
    let feature = snapshot
        .feature(member_id)
        .ok_or(GeneralFabricationError::InvalidWeldmentGeometry)?;
    let FeatureKind::WeldmentMember(spec) = feature.kind() else {
        return Err(GeneralFabricationError::InvalidWeldmentGeometry);
    };
    let path = snapshot
        .feature(spec.path)
        .ok_or(GeneralFabricationError::InvalidWeldmentGeometry)?;
    let FeatureKind::SpatialPath { segments } = path.kind() else {
        return Err(GeneralFabricationError::InvalidWeldmentGeometry);
    };
    let [SpatialPathSegment::Line { start_mm, end_mm }] = segments.as_slice() else {
        return Err(GeneralFabricationError::InvalidWeldmentGeometry);
    };
    Ok((spec.profile, spec.orientation_degrees, *start_mm, *end_mm))
}

fn weldment_distance(left: [f64; 3], right: [f64; 3]) -> f64 {
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2) + (left[2] - right[2]).powi(2))
        .sqrt()
}

fn weldment_joint_member_cut(
    snapshot: &Snapshot,
    member_id: FeatureId,
    joint: &crate::document::WeldmentJointSpec,
) -> Result<Option<(bool, WeldmentEndCut)>, GeneralFabricationError> {
    if joint.first_member != member_id && joint.second_member != member_id {
        return Ok(None);
    }
    let (_, _, first_start, first_end) = straight_weldment_member(snapshot, joint.first_member)?;
    let (_, _, second_start, second_end) = straight_weldment_member(snapshot, joint.second_member)?;
    let endpoint_pairs = [
        (true, first_start, first_end, true, second_start, second_end),
        (
            true,
            first_start,
            first_end,
            false,
            second_end,
            second_start,
        ),
        (
            false,
            first_end,
            first_start,
            true,
            second_start,
            second_end,
        ),
        (
            false,
            first_end,
            first_start,
            false,
            second_end,
            second_start,
        ),
    ];
    let matching = endpoint_pairs
        .into_iter()
        .filter(|(_, first_joint, _, _, second_joint, _)| {
            weldment_distance(*first_joint, *second_joint) <= 1.0e-9
        })
        .collect::<Vec<_>>();
    let [(first_is_start, joint_point, first_far, second_is_start, _, second_far)] =
        matching.as_slice()
    else {
        return Err(GeneralFabricationError::InvalidWeldmentGeometry);
    };
    let first_length = weldment_distance(*joint_point, *first_far);
    let second_length = weldment_distance(*joint_point, *second_far);
    if !first_length.is_finite()
        || !second_length.is_finite()
        || first_length <= 0.0
        || second_length <= 0.0
    {
        return Err(GeneralFabricationError::InvalidWeldmentGeometry);
    }
    let first_direction =
        std::array::from_fn::<_, 3, _>(|axis| (first_far[axis] - joint_point[axis]) / first_length);
    let second_direction = std::array::from_fn::<_, 3, _>(|axis| {
        (second_far[axis] - joint_point[axis]) / second_length
    });
    let dot = first_direction
        .into_iter()
        .zip(second_direction)
        .map(|(left, right)| left * right)
        .sum::<f64>()
        .clamp(-1.0, 1.0);
    let included_angle = dot.acos().to_degrees();
    if !included_angle.is_finite() || !(5.0..=175.0).contains(&included_angle) {
        return Err(GeneralFabricationError::InvalidWeldmentGeometry);
    }
    let is_first = member_id == joint.first_member;
    let is_primary = matches!(
        (is_first, joint.primary),
        (true, WeldmentJointPrimary::First) | (false, WeldmentJointPrimary::Second)
    );
    let cut = match joint.policy {
        WeldmentJointPolicy::Miter => WeldmentEndCut {
            treatment: WeldmentCutTreatment::Miter,
            angle_degrees: included_angle / 2.0,
        },
        WeldmentJointPolicy::Butt if is_primary => WeldmentEndCut {
            treatment: WeldmentCutTreatment::Square,
            angle_degrees: 90.0,
        },
        WeldmentJointPolicy::Butt => WeldmentEndCut {
            treatment: WeldmentCutTreatment::Butt,
            angle_degrees: included_angle.min(180.0 - included_angle),
        },
    };
    Ok(Some((
        if is_first {
            *first_is_start
        } else {
            *second_is_start
        },
        cut,
    )))
}

fn project_weldment_cut_list(
    snapshot: &Snapshot,
    bom: &GeneralBomProjection,
) -> Result<Option<WeldmentCutListProjection>, GeneralFabricationError> {
    let mut rows = Vec::new();
    for bom_row in &bom.rows {
        if bom_row.item_kind != GeneralBomItemKind::Manufactured {
            continue;
        }
        let definition = snapshot
            .definition(bom_row.definition_id)
            .ok_or(GeneralFabricationError::InvalidWeldmentGeometry)?;
        let member_ids = definition
            .feature_ids()
            .iter()
            .copied()
            .filter(|feature_id| {
                matches!(
                    snapshot.feature(*feature_id).map(|feature| feature.kind()),
                    Some(FeatureKind::WeldmentMember(_))
                )
            })
            .collect::<Vec<_>>();
        if member_ids.is_empty() {
            continue;
        }
        for member_id in member_ids {
            let (profile_feature_id, orientation_degrees, start_mm, end_mm) =
                straight_weldment_member(snapshot, member_id)?;
            let centerline_length_mm = weldment_distance(start_mm, end_mm);
            if !centerline_length_mm.is_finite() || centerline_length_mm <= 0.0 {
                return Err(GeneralFabricationError::InvalidWeldmentGeometry);
            }
            let mut start_cut = WeldmentEndCut {
                treatment: WeldmentCutTreatment::Square,
                angle_degrees: 90.0,
            };
            let mut end_cut = start_cut;
            let mut start_joint = false;
            let mut end_joint = false;
            for feature_id in definition.feature_ids() {
                let Some(FeatureKind::WeldmentJoint(joint)) =
                    snapshot.feature(*feature_id).map(|feature| feature.kind())
                else {
                    continue;
                };
                let Some((is_start, cut)) = weldment_joint_member_cut(snapshot, member_id, joint)?
                else {
                    continue;
                };
                let already_assigned = if is_start {
                    std::mem::replace(&mut start_joint, true)
                } else {
                    std::mem::replace(&mut end_joint, true)
                };
                if already_assigned {
                    return Err(GeneralFabricationError::InvalidWeldmentGeometry);
                }
                if is_start {
                    start_cut = cut;
                } else {
                    end_cut = cut;
                }
            }
            rows.push(WeldmentCutListRow {
                stable_row_id: format!(
                    "definition-{}/member-{}/material-{}",
                    bom_row.definition_id.0,
                    member_id.0,
                    &sha256_hex(bom_row.material_key.as_bytes())[..16],
                ),
                position: 0,
                definition_id: bom_row.definition_id,
                member_feature_id: member_id,
                profile_feature_id,
                material_key: bom_row.material_key.clone(),
                quantity: bom_row.quantity,
                centerline_length_mm,
                orientation_degrees,
                start_cut,
                end_cut,
                instances: bom_row.instances.clone(),
                evidence_class: bom_row.evidence_class.clone(),
                validation_state: bom_row.validation_state,
            });
        }
    }
    if rows.is_empty() {
        return Ok(None);
    }
    rows.sort_by(|left, right| {
        left.definition_id
            .cmp(&right.definition_id)
            .then_with(|| left.member_feature_id.cmp(&right.member_feature_id))
            .then_with(|| left.material_key.cmp(&right.material_key))
    });
    if rows
        .iter()
        .map(|row| &row.stable_row_id)
        .collect::<BTreeSet<_>>()
        .len()
        != rows.len()
    {
        return Err(GeneralFabricationError::InvalidWeldmentGeometry);
    }
    for (index, row) in rows.iter_mut().enumerate() {
        row.position = index + 1;
    }
    let validation_state = rows
        .iter()
        .map(|row| row.validation_state)
        .find(|state| *state != ValidationState::Passed)
        .unwrap_or(ValidationState::Passed);
    let status = if validation_state == ValidationState::Passed {
        ProjectionStatus::Complete
    } else {
        ProjectionStatus::Incomplete
    };
    let cut_list_bytes = weldment_cut_list_bytes(&rows, validation_state);
    let cut_list_envelope = FabricationProjectionEnvelope::new_with_evaluator(
        snapshot,
        &cut_list_bytes,
        status,
        WELDMENT_FABRICATION_EVALUATOR_V1,
    );
    let drawing_bytes =
        weldment_drawing_bytes(&rows, &cut_list_envelope.result_digest, validation_state);
    let drawing_envelope = FabricationProjectionEnvelope::new_with_evaluator(
        snapshot,
        &drawing_bytes,
        status,
        WELDMENT_FABRICATION_EVALUATOR_V1,
    );
    Ok(Some(WeldmentCutListProjection {
        cut_list_envelope,
        drawing_envelope,
        validation_state,
        rows,
    }))
}

pub fn project_general_fabrication(
    snapshot: &Snapshot,
    registry: &ExactResultRegistry,
    validation_cases: &[GeneralClearanceCase],
    validation_report: &ValidationReport,
    tolerance: TolerancePolicy,
) -> Result<GeneralFabricationProjection, GeneralFabricationError> {
    let bom_metadata = bom_occurrence_metadata(snapshot)?;
    let validation_input = general_body_input_bytes(validation_cases);
    if !validation_report.invocation.is_current(snapshot)
        || validation_report.invocation.contract_id != GENERAL_BODY_VALIDATOR_CONTRACT_V1
        || validation_report.invocation.input_schema != GENERAL_BODY_VALIDATOR_INPUT_V1
        || validation_report.invocation.input_digest != sha256_hex(&validation_input)
    {
        return Err(GeneralFabricationError::ValidationBindingMismatch);
    }
    let covered = validation_cases
        .iter()
        .flat_map(|case| [&case.left, &case.right])
        .map(|participant| {
            (
                participant.instance_path().clone(),
                participant.source().clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let single_visible_bom_item = validation_cases.is_empty()
        && snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| {
                occurrence.visible && bom_metadata.contains_key(&occurrence.occurrence_id)
            })
            .count()
            == 1;
    let mut accepted = Vec::new();
    for occurrence in snapshot.scene_query().into_iter().filter(|occurrence| {
        occurrence.visible && bom_metadata.contains_key(&occurrence.occurrence_id)
    }) {
        let definition = snapshot
            .definition(occurrence.definition_id)
            .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
        if definition.feature_ids().is_empty() {
            continue;
        }
        if !is_rigid_transform(occurrence.transform) {
            return Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry);
        }
        let participant = GeneralBodyParticipant::accept(
            snapshot,
            registry,
            occurrence.instance_path.clone(),
            tolerance,
        )?;
        if !single_visible_bom_item
            && !covered.contains(&(
                participant.instance_path().clone(),
                participant.source().clone(),
            ))
        {
            return Err(GeneralFabricationError::ValidationBindingMismatch);
        }
        let dimensions = local_dimensions(snapshot, registry, participant.source())?;
        let metadata = bom_metadata
            .get(&occurrence.occurrence_id)
            .expect("the filtered BOM occurrence has metadata")
            .clone();
        accepted.push((participant, dimensions, metadata));
    }
    if accepted.is_empty() {
        return Err(GeneralFabricationError::NoSupportedGeometry);
    }
    accepted.sort_by(|left, right| {
        left.0
            .source()
            .cmp(right.0.source())
            .then_with(|| left.0.instance_path().cmp(right.0.instance_path()))
    });

    let mut evidence_counts = EvidenceCounts::default();
    for (participant, _, _) in &accepted {
        evidence_counts.record(participant.evidence_class());
    }
    let validation_state = validation_report.state;
    let general_status = if validation_state == ValidationState::Passed {
        ProjectionStatus::Complete
    } else {
        ProjectionStatus::Incomplete
    };
    let mut grouped = BTreeMap::<
        (GeneralBodySource, [u64; 3], GeneralBomItemKind, String),
        Vec<&GeneralBodyParticipant>,
    >::new();
    for (participant, dimensions, metadata) in &accepted {
        grouped
            .entry((
                participant.source().clone(),
                [
                    dimensions.length_mm.to_bits(),
                    dimensions.width_mm.to_bits(),
                    dimensions.height_mm.to_bits(),
                ],
                metadata.item_kind,
                metadata.material_key.clone(),
            ))
            .or_default()
            .push(participant);
    }
    let mut rows = Vec::new();
    for ((source, dimension_bits, item_kind, material_key), participants) in grouped {
        let definition_id = source_definition_id(&source);
        let dimensions = PieceDimensions {
            length_mm: f64::from_bits(dimension_bits[0]),
            width_mm: f64::from_bits(dimension_bits[1]),
            height_mm: f64::from_bits(dimension_bits[2]),
        };
        let mut instances = participants
            .iter()
            .map(|participant| participant.instance_path().clone())
            .collect::<Vec<_>>();
        instances.sort();
        let evidence_class = EvidenceClass::weakest(
            participants
                .iter()
                .map(|participant| participant.evidence_class()),
            TolerantEvidence::new(
                tolerance.epsilon_mm(),
                GENERAL_FABRICATION_EVALUATOR_V5,
                PermittedErrorDirection::BidirectionalBounded,
            )
            .expect("the fabrication tolerance and method identity are valid"),
        );
        rows.push(GeneralBomRow {
            stable_row_id: format!(
                "definition-{}/source-{}/kind-{}/material-{}",
                definition_id.0,
                source_digest_token(&source),
                item_kind.token(),
                &sha256_hex(material_key.as_bytes())[..16]
            ),
            position: rows.len() + 1,
            definition_id,
            source,
            item_kind,
            material_key,
            quantity: instances.len(),
            dimensions,
            instances,
            evidence_class,
            validation_state,
        });
    }
    let bom_bytes = general_bom_bytes(&rows, evidence_counts);
    let bom = GeneralBomProjection {
        envelope: FabricationProjectionEnvelope::new_with_evaluator(
            snapshot,
            &bom_bytes,
            general_status,
            GENERAL_FABRICATION_EVALUATOR_V5,
        ),
        evidence_counts,
        rows,
    };

    let mut operations = Vec::new();
    let mut unresolved_sources = Vec::new();
    for row in &bom.rows {
        if row.item_kind != GeneralBomItemKind::Timber {
            continue;
        }
        match &row.source {
            GeneralBodySource::Exact(source) => {
                let package = registry
                    .get_result(source)
                    .filter(|package| package.is_current(snapshot))
                    .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
                if !manufacturing_export_token_is_safe(&source.result_fingerprint) {
                    unresolved_sources.push(row.source.clone());
                    continue;
                }
                let row_operations = match package.as_ref() {
                    ExactBodyPackage::Rectangle(_) => {
                        rectangle_manufacturing_operations(snapshot, row, source)?
                    }
                    ExactBodyPackage::Graph(package) => {
                        graph_manufacturing_operations(row, source, &package.graph)
                    }
                    ExactBodyPackage::Revolve(_) | ExactBodyPackage::Imported(_) => None,
                };
                let Some(row_operations) = row_operations else {
                    unresolved_sources.push(row.source.clone());
                    continue;
                };
                operations.extend(row_operations);
            }
            GeneralBodySource::CanonicalMesh { .. }
            | GeneralBodySource::CanonicalExtrusion { .. }
            | GeneralBodySource::CanonicalExactGraph { .. } => {
                unresolved_sources.push(row.source.clone());
            }
        }
    }
    unresolved_sources.sort();
    unresolved_sources.dedup();
    let manufacturing_status =
        if validation_state == ValidationState::Passed && unresolved_sources.is_empty() {
            ProjectionStatus::Complete
        } else {
            ProjectionStatus::Incomplete
        };
    let manufacturing_bytes =
        general_manufacturing_bytes(&operations, &unresolved_sources, validation_state);
    let manufacturing = GeneralManufacturingProjection {
        envelope: FabricationProjectionEnvelope::new_with_evaluator(
            snapshot,
            &manufacturing_bytes,
            manufacturing_status,
            GENERAL_FABRICATION_EVALUATOR_V5,
        ),
        validation_state,
        operations,
        unresolved_sources,
    };
    let drawings = bom
        .rows
        .iter()
        .map(|row| general_piece_drawing(row, &manufacturing.operations))
        .collect::<Vec<_>>();
    let drawing_bytes = general_drawing_bytes(&drawings, validation_state);
    let drawings = GeneralDrawingProjection {
        envelope: FabricationProjectionEnvelope::new_with_evaluator(
            snapshot,
            &drawing_bytes,
            general_status,
            GENERAL_FABRICATION_EVALUATOR_V5,
        ),
        validation_state,
        drawings,
    };
    let weldment = project_weldment_cut_list(snapshot, &bom)?;
    Ok(GeneralFabricationProjection {
        bom,
        drawings,
        manufacturing,
        weldment,
    })
}

fn rectangle_manufacturing_operations(
    snapshot: &Snapshot,
    row: &GeneralBomRow,
    source: &crate::exact_product::ExactResultKey,
) -> Result<Option<Vec<GeneralManufacturingOperation>>, GeneralFabricationError> {
    let definition = snapshot
        .definition(row.definition_id)
        .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
    if definition.feature_ids().iter().any(|feature_id| {
        matches!(
            snapshot.feature(*feature_id).map(|feature| feature.kind()),
            Some(FeatureKind::Boolean {
                operation: BooleanOperation::Union,
                ..
            })
        )
    }) {
        return Ok(None);
    }
    let mut operations = vec![GeneralManufacturingOperation {
        stable_operation_id: format!("definition-{}/stock", row.definition_id.0),
        definition_id: row.definition_id,
        producer_feature_id: source.producer_feature_id,
        kind: GeneralManufacturingKind::Stock,
        semantic_inputs: Vec::new(),
        frame: "definition-local",
        bounds: row.dimensions,
        machining: {
            let graph = ExactBRepGraph::from_snapshot(
                snapshot,
                row.definition_id,
                source.producer_feature_id,
            )
            .map_err(|_| GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
            let Some(stock) = graph.nodes.first() else {
                return Ok(None);
            };
            let ExactBRepOperation::Extrude {
                profile, interval, ..
            } = stock.operation
            else {
                return Ok(None);
            };
            let Some(geometry) = graph
                .profiles
                .get(profile.0 as usize)
                .and_then(|profile| timber_stock_geometry(profile, interval))
            else {
                return Ok(None);
            };
            geometry
        },
        source: source.clone(),
    }];
    for feature_id in definition.feature_ids() {
        let feature = snapshot
            .feature(*feature_id)
            .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
        let (kind, semantic_inputs, machining) = match feature.kind() {
            FeatureKind::ThroughCut { target, profile } => (
                GeneralManufacturingKind::ThroughCut,
                vec![*target, *profile],
                legacy_profile_cut_geometry(snapshot, *profile, row.dimensions, true)?,
            ),
            FeatureKind::Pocket {
                target,
                profile,
                depth,
            } => (
                GeneralManufacturingKind::ProfileCut,
                vec![*target, *profile],
                legacy_profile_cut_geometry(
                    snapshot,
                    *profile,
                    PieceDimensions {
                        height_mm: depth.millimetres(),
                        ..row.dimensions
                    },
                    false,
                )?,
            ),
            FeatureKind::Boolean {
                operation: BooleanOperation::Cut,
                target,
                tool,
            } => (
                GeneralManufacturingKind::BooleanCut,
                vec![*target, *tool],
                legacy_boolean_cut_geometry(snapshot, *tool)?,
            ),
            _ => continue,
        };
        operations.push(GeneralManufacturingOperation {
            stable_operation_id: format!(
                "definition-{}/feature-{}/{}",
                row.definition_id.0,
                feature_id.0,
                kind.token()
            ),
            definition_id: row.definition_id,
            producer_feature_id: *feature_id,
            kind,
            semantic_inputs,
            frame: "definition-local",
            bounds: row.dimensions,
            machining,
            source: source.clone(),
        });
    }
    Ok(Some(operations))
}

fn graph_manufacturing_operations(
    row: &GeneralBomRow,
    source: &crate::exact_product::ExactResultKey,
    graph: &ExactBRepGraph,
) -> Option<Vec<GeneralManufacturingOperation>> {
    let stock = graph.nodes.first()?;
    let ExactBRepOperation::Extrude {
        profile: stock_profile,
        interval: stock_interval,
        ..
    } = stock.operation
    else {
        return None;
    };
    let stock_bounds =
        piece_dimensions_from_bounds(graph.node_bounds_mm(stock.id).ok().flatten()?)?;
    let mut operations = vec![GeneralManufacturingOperation {
        stable_operation_id: format!("definition-{}/stock", row.definition_id.0),
        definition_id: row.definition_id,
        producer_feature_id: FeatureId(stock.source_feature_id),
        kind: GeneralManufacturingKind::Stock,
        semantic_inputs: Vec::new(),
        frame: "definition-local",
        bounds: stock_bounds,
        machining: timber_stock_geometry(
            graph.profiles.get(stock_profile.0 as usize)?,
            stock_interval,
        )?,
        source: source.clone(),
    }];
    if graph.nodes.len() == 1 {
        return Some(operations);
    }

    let mut previous = stock;
    let mut profile_cut_chain = true;
    for node in graph.nodes.iter().skip(1) {
        let ExactBRepOperation::ProfileCut {
            target,
            profile,
            depth_bits,
            interval,
            ..
        } = node.operation
        else {
            profile_cut_chain = false;
            break;
        };
        if target != previous.id {
            return None;
        }
        let profile = graph.profiles.get(profile.0 as usize)?;
        let machining = profile_cut_geometry(profile, interval)?;
        let kind = if matches!(machining, GeneralMachiningGeometry::CircularDrill { .. }) {
            GeneralManufacturingKind::CircularDrill
        } else if depth_bits.is_none() {
            GeneralManufacturingKind::ThroughCut
        } else {
            GeneralManufacturingKind::ProfileCut
        };
        operations.push(GeneralManufacturingOperation {
            stable_operation_id: format!(
                "definition-{}/feature-{}/{}",
                row.definition_id.0,
                node.source_feature_id,
                kind.token()
            ),
            definition_id: row.definition_id,
            producer_feature_id: FeatureId(node.source_feature_id),
            kind,
            semantic_inputs: vec![
                FeatureId(previous.source_feature_id),
                FeatureId(profile.source_feature_id),
            ],
            frame: "definition-local",
            bounds: row.dimensions,
            machining,
            source: source.clone(),
        });
        previous = node;
    }
    if profile_cut_chain {
        return Some(operations);
    }

    let [target, tool, terminal] = graph.nodes.as_slice() else {
        return None;
    };
    let ExactBRepOperation::Extrude {
        profile: tool_profile,
        interval: tool_interval,
        ..
    } = tool.operation
    else {
        return None;
    };
    let ExactBRepOperation::Boolean {
        operation: ExactBRepBooleanOperation::Cut,
        target: boolean_target,
        tool: boolean_tool,
    } = terminal.operation
    else {
        return None;
    };
    if boolean_target != target.id || boolean_tool != tool.id || target.id == tool.id {
        return None;
    }
    operations.push(GeneralManufacturingOperation {
        stable_operation_id: format!(
            "definition-{}/feature-{}/{}",
            row.definition_id.0,
            terminal.source_feature_id,
            GeneralManufacturingKind::BooleanCut.token()
        ),
        definition_id: row.definition_id,
        producer_feature_id: FeatureId(terminal.source_feature_id),
        kind: GeneralManufacturingKind::BooleanCut,
        semantic_inputs: vec![
            FeatureId(target.source_feature_id),
            FeatureId(tool.source_feature_id),
        ],
        frame: "definition-local",
        bounds: row.dimensions,
        machining: profile_cut_geometry(
            graph.profiles.get(tool_profile.0 as usize)?,
            tool_interval,
        )?,
        source: source.clone(),
    });
    Some(operations)
}

fn legacy_profile_cut_geometry(
    snapshot: &Snapshot,
    profile_id: FeatureId,
    dimensions: PieceDimensions,
    _through: bool,
) -> Result<GeneralMachiningGeometry, GeneralFabricationError> {
    let feature = snapshot
        .feature(profile_id)
        .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
    let segments = match feature.kind() {
        FeatureKind::Profile { points_mm } if points_mm.len() >= 3 => points_mm
            .iter()
            .zip(points_mm.iter().cycle().skip(1))
            .take(points_mm.len())
            .map(|(start_mm, end_mm)| GeneralMachiningSegment::Line {
                start_mm: *start_mm,
                end_mm: *end_mm,
            })
            .collect(),
        _ => return Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry),
    };
    Ok(GeneralMachiningGeometry::ProfileCut {
        frame: identity_machining_frame(),
        segments,
        start_mm: 0.0,
        end_mm: dimensions.height_mm,
    })
}

fn legacy_boolean_cut_geometry(
    snapshot: &Snapshot,
    tool_id: FeatureId,
) -> Result<GeneralMachiningGeometry, GeneralFabricationError> {
    let FeatureKind::Extrusion { profile, height } = snapshot
        .feature(tool_id)
        .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?
        .kind()
    else {
        return Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry);
    };
    legacy_profile_cut_geometry(
        snapshot,
        *profile,
        PieceDimensions {
            length_mm: 1.0,
            width_mm: 1.0,
            height_mm: height.millimetres(),
        },
        false,
    )
}

fn timber_stock_geometry(
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Option<GeneralMachiningGeometry> {
    let frame = machining_frame(profile, interval.direction())?;
    let ExactBRepPlanarGeometry::Boundary {
        closed: true,
        segments,
    } = &profile.geometry
    else {
        return None;
    };
    let mut points = Vec::new();
    let mut cross_section = Vec::new();
    for segment in segments {
        let ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } = segment
        else {
            return None;
        };
        let start_mm = start_bits.map(f64::from_bits);
        let end_mm = end_bits.map(f64::from_bits);
        points.push(start_mm);
        points.push(end_mm);
        cross_section.push(GeneralMachiningSegment::Line { start_mm, end_mm });
    }
    let minimum = [
        points
            .iter()
            .map(|point| point[0])
            .fold(f64::INFINITY, f64::min),
        points
            .iter()
            .map(|point| point[1])
            .fold(f64::INFINITY, f64::min),
    ];
    let maximum = [
        points
            .iter()
            .map(|point| point[0])
            .fold(f64::NEG_INFINITY, f64::max),
        points
            .iter()
            .map(|point| point[1])
            .fold(f64::NEG_INFINITY, f64::max),
    ];
    let width = maximum[0] - minimum[0];
    let height = maximum[1] - minimum[1];
    let direction = interval.direction();
    let length = interval.length_mm();
    if !width.is_finite()
        || !height.is_finite()
        || !length.is_finite()
        || width <= 0.0
        || height <= 0.0
        || length <= 0.0
    {
        return None;
    }
    Some(GeneralMachiningGeometry::TimberStock {
        frame,
        cross_section,
        start_mm: std::array::from_fn(|axis| {
            frame.origin_mm[axis] + direction[axis] * interval.start_mm()
        }),
        length_axis: direction,
        length_mm: length,
        cross_section_width_mm: width,
        cross_section_height_mm: height,
    })
}

fn profile_cut_geometry(
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Option<GeneralMachiningGeometry> {
    let frame = machining_frame(profile, interval.direction())?;
    if let Some((center_mm, radius_mm)) = circular_profile(&profile.geometry) {
        return Some(GeneralMachiningGeometry::CircularDrill {
            frame,
            center_mm,
            diameter_mm: radius_mm * 2.0,
            start_mm: interval.start_mm(),
            end_mm: interval.end_mm(),
        });
    }
    let ExactBRepPlanarGeometry::Boundary {
        closed: true,
        segments,
    } = &profile.geometry
    else {
        return None;
    };
    let segments = segments
        .iter()
        .map(|segment| match segment {
            ExactBRepPlanarSegment::Line {
                start_bits,
                end_bits,
            } => Some(GeneralMachiningSegment::Line {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            }),
            ExactBRepPlanarSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => Some(GeneralMachiningSegment::CircularArc {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
                center_mm: center_bits.map(f64::from_bits),
                clockwise: *clockwise,
            }),
            ExactBRepPlanarSegment::CubicBezier { .. } => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some(GeneralMachiningGeometry::ProfileCut {
        frame,
        segments,
        start_mm: interval.start_mm(),
        end_mm: interval.end_mm(),
    })
}

fn machining_frame(
    profile: &ExactBRepProfile,
    interval_direction: [f64; 3],
) -> Option<GeneralMachiningFrame> {
    let values = profile.frame_bits.map(f64::from_bits);
    if !values.iter().all(|value| value.is_finite())
        || !interval_direction.iter().all(|value| value.is_finite())
    {
        return None;
    }
    let frame = GeneralMachiningFrame {
        origin_mm: values[0..3].try_into().ok()?,
        x_axis: values[3..6].try_into().ok()?,
        y_axis: values[6..9].try_into().ok()?,
        normal: values[9..12].try_into().ok()?,
    };
    let dot = |left: [f64; 3], right: [f64; 3]| {
        left.into_iter()
            .zip(right)
            .map(|(left, right)| left * right)
            .sum::<f64>()
    };
    let cross = [
        frame.x_axis[1] * frame.y_axis[2] - frame.x_axis[2] * frame.y_axis[1],
        frame.x_axis[2] * frame.y_axis[0] - frame.x_axis[0] * frame.y_axis[2],
        frame.x_axis[0] * frame.y_axis[1] - frame.x_axis[1] * frame.y_axis[0],
    ];
    let close = |left: f64, right: f64| (left - right).abs() <= 1.0e-12;
    (close(dot(frame.x_axis, frame.x_axis), 1.0)
        && close(dot(frame.y_axis, frame.y_axis), 1.0)
        && close(dot(frame.normal, frame.normal), 1.0)
        && close(dot(frame.x_axis, frame.y_axis), 0.0)
        && cross
            .into_iter()
            .zip(frame.normal)
            .all(|(actual, expected)| close(actual, expected))
        && frame
            .normal
            .into_iter()
            .zip(interval_direction)
            .all(|(normal, direction)| close(normal, direction)))
    .then_some(frame)
}

const fn identity_machining_frame() -> GeneralMachiningFrame {
    GeneralMachiningFrame {
        origin_mm: [0.0, 0.0, 0.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    }
}

fn circular_profile(geometry: &ExactBRepPlanarGeometry) -> Option<([f64; 2], f64)> {
    if let ExactBRepPlanarGeometry::Circle {
        center_bits,
        radius_bits,
    } = geometry
    {
        let center = center_bits.map(f64::from_bits);
        let radius = f64::from_bits(*radius_bits);
        return (center.iter().all(|value| value.is_finite())
            && radius.is_finite()
            && radius > 0.0)
            .then_some((center, radius));
    }
    let ExactBRepPlanarGeometry::Boundary {
        closed: true,
        segments,
    } = geometry
    else {
        return None;
    };
    if segments.len() != 4 {
        return None;
    }
    let ExactBRepPlanarSegment::CircularArc {
        start_bits,
        center_bits,
        clockwise,
        ..
    } = &segments[0]
    else {
        return None;
    };
    let center = center_bits.map(f64::from_bits);
    let start = start_bits.map(f64::from_bits);
    let radius_squared = (start[0] - center[0]).powi(2) + (start[1] - center[1]).powi(2);
    let close = |left: f64, right: f64| {
        (left - right).abs() <= 1.0e-12 * left.abs().max(right.abs()).max(1.0)
    };
    let mut previous_end = start;
    for segment in segments {
        let ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise: segment_clockwise,
        } = segment
        else {
            return None;
        };
        let segment_start = start_bits.map(f64::from_bits);
        let segment_end = end_bits.map(f64::from_bits);
        let segment_center = center_bits.map(f64::from_bits);
        let start_vector = [segment_start[0] - center[0], segment_start[1] - center[1]];
        let end_vector = [segment_end[0] - center[0], segment_end[1] - center[1]];
        let end_radius_squared = end_vector[0].powi(2) + end_vector[1].powi(2);
        let dot = start_vector[0] * end_vector[0] + start_vector[1] * end_vector[1];
        let cross = start_vector[0] * end_vector[1] - start_vector[1] * end_vector[0];
        if segment_center != center
            || segment_start != previous_end
            || segment_clockwise != clockwise
            || !close(end_radius_squared, radius_squared)
            || !close(dot, 0.0)
            || !close(cross.abs(), radius_squared)
            || (*clockwise && cross >= 0.0)
            || (!*clockwise && cross <= 0.0)
        {
            return None;
        }
        previous_end = segment_end;
    }
    (previous_end == start
        && center.iter().all(|value| value.is_finite())
        && radius_squared.is_finite()
        && radius_squared > 0.0)
        .then_some((center, radius_squared.sqrt()))
}

fn machining_detail_bounds(geometry: &GeneralMachiningGeometry) -> Option<([f64; 2], [f64; 2])> {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    let mut include = |point: [f64; 2]| {
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    };
    match geometry {
        GeneralMachiningGeometry::TimberStock { .. } => return None,
        GeneralMachiningGeometry::ProfileCut { segments, .. } => {
            for segment in segments {
                match segment {
                    GeneralMachiningSegment::Line { start_mm, end_mm } => {
                        include(*start_mm);
                        include(*end_mm);
                    }
                    GeneralMachiningSegment::CircularArc {
                        start_mm,
                        end_mm,
                        center_mm,
                        ..
                    } => {
                        let radius = ((start_mm[0] - center_mm[0]).powi(2)
                            + (start_mm[1] - center_mm[1]).powi(2))
                        .sqrt();
                        include(*start_mm);
                        include(*end_mm);
                        include([center_mm[0] - radius, center_mm[1] - radius]);
                        include([center_mm[0] + radius, center_mm[1] + radius]);
                    }
                }
            }
        }
        GeneralMachiningGeometry::CircularDrill {
            center_mm,
            diameter_mm,
            ..
        } => {
            let radius = diameter_mm / 2.0;
            include([center_mm[0] - radius, center_mm[1] - radius]);
            include([center_mm[0] + radius, center_mm[1] + radius]);
        }
    }
    let size = [maximum[0] - minimum[0], maximum[1] - minimum[1]];
    (minimum
        .into_iter()
        .chain(maximum)
        .chain(size)
        .all(f64::is_finite)
        && size.into_iter().all(|value| value > 0.0))
    .then_some((minimum, maximum))
}

fn append_machining_detail(
    svg: &mut String,
    operation: &GeneralManufacturingOperation,
    minimum: [f64; 2],
    maximum: [f64; 2],
    x: f64,
    y: f64,
) {
    let point = |value: [f64; 2]| [x + value[0] - minimum[0], y + value[1] - minimum[1]];
    let width = maximum[0] - minimum[0];
    let height = maximum[1] - minimum[1];
    svg.push_str(&format!(
        "<g id=\"{}/machining-detail\" data-kind=\"{}\">\n",
        operation.stable_operation_id,
        operation.kind.token()
    ));
    match &operation.machining {
        GeneralMachiningGeometry::ProfileCut {
            segments,
            start_mm,
            end_mm,
            ..
        } => {
            let mut path = String::new();
            for (index, segment) in segments.iter().enumerate() {
                match segment {
                    GeneralMachiningSegment::Line { start_mm, end_mm } => {
                        let start = point(*start_mm);
                        let end = point(*end_mm);
                        if index == 0 {
                            path.push_str(&format!(
                                "M {} {} ",
                                format_number(start[0]),
                                format_number(start[1])
                            ));
                        }
                        path.push_str(&format!(
                            "L {} {} ",
                            format_number(end[0]),
                            format_number(end[1])
                        ));
                    }
                    GeneralMachiningSegment::CircularArc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => {
                        let start = point(*start_mm);
                        let end = point(*end_mm);
                        let radius = ((start_mm[0] - center_mm[0]).powi(2)
                            + (start_mm[1] - center_mm[1]).powi(2))
                        .sqrt();
                        if index == 0 {
                            path.push_str(&format!(
                                "M {} {} ",
                                format_number(start[0]),
                                format_number(start[1])
                            ));
                        }
                        let start_angle =
                            (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
                        let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
                        let mut sweep = end_angle - start_angle;
                        if *clockwise {
                            while sweep < 0.0 {
                                sweep += std::f64::consts::TAU;
                            }
                        } else {
                            while sweep > 0.0 {
                                sweep -= std::f64::consts::TAU;
                            }
                        }
                        path.push_str(&format!(
                            "A {0} {0} 0 {1} {2} {3} {4} ",
                            format_number(radius),
                            u8::from(sweep.abs() > std::f64::consts::PI),
                            u8::from(*clockwise),
                            format_number(end[0]),
                            format_number(end[1])
                        ));
                    }
                }
            }
            path.push('Z');
            svg.push_str(&format!(
                "<path id=\"{}/geometry\" d=\"{}\" fill=\"white\" />\n<text x=\"{}\" y=\"{}\" fill=\"black\" stroke=\"none\">{}: width={} mm, height={} mm, depth={} mm</text>\n",
                operation.stable_operation_id,
                path,
                format_number(x),
                format_number(y + height + 20.0),
                operation.kind.token(),
                format_number(width),
                format_number(height),
                format_number((end_mm - start_mm).abs())
            ));
        }
        GeneralMachiningGeometry::CircularDrill {
            center_mm,
            diameter_mm,
            start_mm,
            end_mm,
            ..
        } => {
            let center = point(*center_mm);
            svg.push_str(&format!(
                "<circle id=\"{}/geometry\" cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"white\" />\n<text x=\"{}\" y=\"{}\" fill=\"black\" stroke=\"none\">circular-drill: center=({}, {}) mm, diameter={} mm, depth={} mm</text>\n",
                operation.stable_operation_id,
                format_number(center[0]),
                format_number(center[1]),
                format_number(diameter_mm / 2.0),
                format_number(x),
                format_number(y + height + 20.0),
                format_number(center_mm[0]),
                format_number(center_mm[1]),
                format_number(*diameter_mm),
                format_number((end_mm - start_mm).abs())
            ));
        }
        GeneralMachiningGeometry::TimberStock { .. } => unreachable!(),
    }
    svg.push_str("</g>\n");
}

fn machining_token(geometry: &GeneralMachiningGeometry) -> String {
    match geometry {
        GeneralMachiningGeometry::TimberStock {
            frame,
            cross_section,
            start_mm,
            length_axis,
            length_mm,
            cross_section_width_mm,
            cross_section_height_mm,
        } => format!(
            "timber-stock:frame({}):section({}):start({}):axis({}):length({}):cross({},{})",
            frame_token(frame),
            cross_section
                .iter()
                .map(machining_segment_token)
                .collect::<Vec<_>>()
                .join("|"),
            vector_token(start_mm),
            vector_token(length_axis),
            format_number(*length_mm),
            format_number(*cross_section_width_mm),
            format_number(*cross_section_height_mm)
        ),
        GeneralMachiningGeometry::ProfileCut {
            frame,
            segments,
            start_mm,
            end_mm,
        } => format!(
            "profile-cut:frame({}):interval({},{}):segments({})",
            frame_token(frame),
            format_number(*start_mm),
            format_number(*end_mm),
            segments
                .iter()
                .map(machining_segment_token)
                .collect::<Vec<_>>()
                .join("|")
        )
        .replace(", ", ","),
        GeneralMachiningGeometry::CircularDrill {
            frame,
            center_mm,
            diameter_mm,
            start_mm,
            end_mm,
        } => format!(
            "circular-drill:frame({}):center({},{}):diameter({}):interval({},{})",
            frame_token(frame),
            format_number(center_mm[0]),
            format_number(center_mm[1]),
            format_number(*diameter_mm),
            format_number(*start_mm),
            format_number(*end_mm)
        ),
    }
}

fn frame_token(frame: &GeneralMachiningFrame) -> String {
    format!(
        "{}/{}/{}/{}",
        vector_token(&frame.origin_mm),
        vector_token(&frame.x_axis),
        vector_token(&frame.y_axis),
        vector_token(&frame.normal)
    )
}

fn vector_token<const N: usize>(values: &[f64; N]) -> String {
    values
        .iter()
        .map(|value| format_number(*value))
        .collect::<Vec<_>>()
        .join(",")
}

fn machining_segment_token(segment: &GeneralMachiningSegment) -> String {
    match segment {
        GeneralMachiningSegment::Line { start_mm, end_mm } => format!(
            "line({},{},{},{})",
            format_number(start_mm[0]),
            format_number(start_mm[1]),
            format_number(end_mm[0]),
            format_number(end_mm[1])
        ),
        GeneralMachiningSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => format!(
            "arc({},{},{},{},{},{},{})",
            format_number(start_mm[0]),
            format_number(start_mm[1]),
            format_number(end_mm[0]),
            format_number(end_mm[1]),
            format_number(center_mm[0]),
            format_number(center_mm[1]),
            u8::from(*clockwise)
        ),
    }
}

fn push_machining_geometry(bytes: &mut Vec<u8>, geometry: &GeneralMachiningGeometry) {
    match geometry {
        GeneralMachiningGeometry::TimberStock {
            frame,
            cross_section,
            start_mm,
            length_axis,
            length_mm,
            cross_section_width_mm,
            cross_section_height_mm,
        } => {
            bytes.push(0);
            push_machining_frame(bytes, frame);
            push_machining_segments(bytes, cross_section);
            push_f64_array(bytes, start_mm);
            push_f64_array(bytes, length_axis);
            bytes.extend_from_slice(&length_mm.to_bits().to_le_bytes());
            bytes.extend_from_slice(&cross_section_width_mm.to_bits().to_le_bytes());
            bytes.extend_from_slice(&cross_section_height_mm.to_bits().to_le_bytes());
        }
        GeneralMachiningGeometry::ProfileCut {
            frame,
            segments,
            start_mm,
            end_mm,
        } => {
            bytes.push(1);
            push_machining_frame(bytes, frame);
            push_machining_segments(bytes, segments);
            bytes.extend_from_slice(&start_mm.to_bits().to_le_bytes());
            bytes.extend_from_slice(&end_mm.to_bits().to_le_bytes());
        }
        GeneralMachiningGeometry::CircularDrill {
            frame,
            center_mm,
            diameter_mm,
            start_mm,
            end_mm,
        } => {
            bytes.push(2);
            push_machining_frame(bytes, frame);
            push_f64_array(bytes, center_mm);
            bytes.extend_from_slice(&diameter_mm.to_bits().to_le_bytes());
            bytes.extend_from_slice(&start_mm.to_bits().to_le_bytes());
            bytes.extend_from_slice(&end_mm.to_bits().to_le_bytes());
        }
    }
}

fn push_machining_frame(bytes: &mut Vec<u8>, frame: &GeneralMachiningFrame) {
    push_f64_array(bytes, &frame.origin_mm);
    push_f64_array(bytes, &frame.x_axis);
    push_f64_array(bytes, &frame.y_axis);
    push_f64_array(bytes, &frame.normal);
}

fn push_machining_segments(bytes: &mut Vec<u8>, segments: &[GeneralMachiningSegment]) {
    bytes.extend_from_slice(&(segments.len() as u64).to_le_bytes());
    for segment in segments {
        match segment {
            GeneralMachiningSegment::Line { start_mm, end_mm } => {
                bytes.push(0);
                push_f64_array(bytes, start_mm);
                push_f64_array(bytes, end_mm);
            }
            GeneralMachiningSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                bytes.push(1);
                push_f64_array(bytes, start_mm);
                push_f64_array(bytes, end_mm);
                push_f64_array(bytes, center_mm);
                bytes.push(u8::from(*clockwise));
            }
        }
    }
}

fn push_f64_array<const N: usize>(bytes: &mut Vec<u8>, values: &[f64; N]) {
    for value in values {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}

fn manufacturing_export_token_is_safe(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|character| !character.is_control() && character != ';' && character != '=')
}

fn piece_dimensions_from_bounds(bounds: [[f64; 3]; 2]) -> Option<PieceDimensions> {
    let dimensions = PieceDimensions {
        length_mm: bounds[1][0] - bounds[0][0],
        width_mm: bounds[1][1] - bounds[0][1],
        height_mm: bounds[1][2] - bounds[0][2],
    };
    [
        dimensions.length_mm,
        dimensions.width_mm,
        dimensions.height_mm,
    ]
    .into_iter()
    .all(|value| value.is_finite() && value > 0.0)
    .then_some(dimensions)
}

fn weldment_envelope_is_current(
    envelope: &FabricationProjectionEnvelope,
    snapshot: &Snapshot,
) -> bool {
    envelope.projection_schema == FABRICATION_PROJECTION_V1
        && envelope.evaluator_id == WELDMENT_FABRICATION_EVALUATOR_V1
        && envelope.is_current(snapshot)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn general_envelope_is_current(
    envelope: &FabricationProjectionEnvelope,
    snapshot: &Snapshot,
) -> bool {
    envelope.projection_schema == FABRICATION_PROJECTION_V1
        && envelope.evaluator_id == GENERAL_FABRICATION_EVALUATOR_V5
        && envelope.is_current(snapshot)
}

fn is_production_transform(transform: Transform) -> bool {
    let m = transform.matrix();
    let determinant = m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8])
        + m[2] * (m[4] * m[9] - m[5] * m[8]);
    is_rigid_transform(transform) && (determinant - 1.0).abs() <= 1.0e-12
}
fn is_rigid_transform(transform: Transform) -> bool {
    let matrix = transform.matrix();
    if matrix[12] != 0.0 || matrix[13] != 0.0 || matrix[14] != 0.0 || matrix[15] != 1.0 {
        return false;
    }
    let columns = [
        [matrix[0], matrix[4], matrix[8]],
        [matrix[1], matrix[5], matrix[9]],
        [matrix[2], matrix[6], matrix[10]],
    ];
    let dot = |left: [f64; 3], right: [f64; 3]| {
        left.into_iter()
            .zip(right)
            .map(|(left, right)| left * right)
            .sum::<f64>()
    };
    let epsilon = 1.0e-12;
    columns
        .iter()
        .all(|column| (dot(*column, *column) - 1.0).abs() <= epsilon)
        && (dot(columns[0], columns[1])).abs() <= epsilon
        && (dot(columns[0], columns[2])).abs() <= epsilon
        && (dot(columns[1], columns[2])).abs() <= epsilon
}

fn local_dimensions(
    snapshot: &Snapshot,
    registry: &ExactResultRegistry,
    source: &GeneralBodySource,
) -> Result<PieceDimensions, GeneralFabricationError> {
    let vertices = match source {
        GeneralBodySource::Exact(key) => registry
            .get_result(key)
            .filter(|package| package.is_current(snapshot))
            .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?
            .vertices()
            .iter()
            .map(|vertex| vertex.position_mm)
            .collect::<Vec<_>>(),
        GeneralBodySource::CanonicalMesh {
            definition_id,
            feature_id,
            ..
        } => {
            let FeatureKind::MeshBody(spec) = snapshot
                .feature(*feature_id)
                .filter(|feature| feature.definition_id() == *definition_id)
                .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?
                .kind()
            else {
                return Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry);
            };
            spec.vertices_mm.clone()
        }
        GeneralBodySource::CanonicalExactGraph {
            definition_id,
            producer_feature_id,
            graph_digest,
        } => {
            let graph =
                ExactBRepGraph::from_snapshot(snapshot, *definition_id, *producer_feature_id)
                    .map_err(|_| GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
            if graph.graph_digest != *graph_digest {
                return Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry);
            }
            let [minimum, maximum] = graph
                .producer_bounds_mm()
                .map_err(|_| GeneralFabricationError::InvalidGeometry)?
                .ok_or(GeneralFabricationError::InvalidGeometry)?;
            vec![minimum, maximum]
        }
        GeneralBodySource::CanonicalExtrusion {
            definition_id,
            profile_id,
            extrusion_id,
            ..
        } => {
            let FeatureKind::Profile { points_mm } = snapshot
                .feature(*profile_id)
                .filter(|feature| feature.definition_id() == *definition_id)
                .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?
                .kind()
            else {
                return Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry);
            };
            let FeatureKind::Extrusion { profile, height } = snapshot
                .feature(*extrusion_id)
                .filter(|feature| feature.definition_id() == *definition_id)
                .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?
                .kind()
            else {
                return Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry);
            };
            if profile != profile_id {
                return Err(GeneralFabricationError::UnsupportedOrUnavailableGeometry);
            }
            points_mm
                .iter()
                .flat_map(|point| {
                    [
                        [point[0], point[1], 0.0],
                        [point[0], point[1], height.millimetres()],
                    ]
                })
                .collect()
        }
    };
    let first = vertices
        .first()
        .copied()
        .ok_or(GeneralFabricationError::InvalidGeometry)?;
    let mut minimum = first;
    let mut maximum = first;
    for vertex in vertices {
        if vertex.iter().any(|coordinate| !coordinate.is_finite()) {
            return Err(GeneralFabricationError::InvalidGeometry);
        }
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(vertex[axis]);
            maximum[axis] = maximum[axis].max(vertex[axis]);
        }
    }
    let dimensions = PieceDimensions {
        length_mm: maximum[0] - minimum[0],
        width_mm: maximum[1] - minimum[1],
        height_mm: maximum[2] - minimum[2],
    };
    if [
        dimensions.length_mm,
        dimensions.width_mm,
        dimensions.height_mm,
    ]
    .into_iter()
    .any(|value| !value.is_finite() || value <= 0.0)
    {
        return Err(GeneralFabricationError::InvalidGeometry);
    }
    Ok(dimensions)
}

fn general_piece_drawing(
    row: &GeneralBomRow,
    operations: &[GeneralManufacturingOperation],
) -> GeneralPieceDrawing {
    let drawing_id = format!("{}/drawing", row.stable_row_id);
    let view = |name, horizontal_axis, vertical_axis, width_mm, height_mm| GeneralDrawingView {
        stable_view_id: format!("{drawing_id}/{name}"),
        name,
        horizontal_axis,
        vertical_axis,
        width_mm,
        height_mm,
    };
    let dimension = |axis, value_mm| GeneralDimensionCallout {
        stable_dimension_id: format!("{drawing_id}/dimension-{axis}"),
        axis,
        value_mm,
    };
    GeneralPieceDrawing {
        stable_drawing_id: drawing_id.clone(),
        bom_row_id: row.stable_row_id.clone(),
        position: row.position,
        definition_id: row.definition_id,
        source: row.source.clone(),
        item_kind: row.item_kind,
        material_key: row.material_key.clone(),
        quantity: row.quantity,
        instances: row.instances.clone(),
        projection_method: "accepted-body-local-bounds",
        views: vec![
            view(
                "front",
                "x",
                "z",
                row.dimensions.length_mm,
                row.dimensions.height_mm,
            ),
            view(
                "top",
                "x",
                "y",
                row.dimensions.length_mm,
                row.dimensions.width_mm,
            ),
            view(
                "right",
                "y",
                "z",
                row.dimensions.width_mm,
                row.dimensions.height_mm,
            ),
        ],
        dimensions: vec![
            dimension("x", row.dimensions.length_mm),
            dimension("y", row.dimensions.width_mm),
            dimension("z", row.dimensions.height_mm),
        ],
        machining_operations: operations
            .iter()
            .filter(|operation| {
                row.item_kind == GeneralBomItemKind::Timber
                    && operation.definition_id == row.definition_id
                    && GeneralBodySource::Exact(operation.source.clone()) == row.source
            })
            .cloned()
            .collect(),
        evidence_class: row.evidence_class.clone(),
    }
}

fn source_definition_id(source: &GeneralBodySource) -> DefinitionId {
    match source {
        GeneralBodySource::Exact(key) => key.definition_id,
        GeneralBodySource::CanonicalMesh { definition_id, .. }
        | GeneralBodySource::CanonicalExtrusion { definition_id, .. }
        | GeneralBodySource::CanonicalExactGraph { definition_id, .. } => *definition_id,
    }
}

fn source_digest_token(source: &GeneralBodySource) -> String {
    let mut bytes = Vec::new();
    push_general_source(&mut bytes, source);
    sha256_hex(&bytes)[..16].to_owned()
}

fn weldment_cut_list_bytes(
    rows: &[WeldmentCutListRow],
    validation_state: ValidationState,
) -> Vec<u8> {
    let mut bytes = WELDMENT_CUT_LIST_EXPORT_V1.as_bytes().to_vec();
    push_validation_state(&mut bytes, validation_state);
    bytes.extend_from_slice(&(rows.len() as u64).to_le_bytes());
    for row in rows {
        push_projection_bytes(&mut bytes, row.stable_row_id.as_bytes());
        bytes.extend_from_slice(&(row.position as u64).to_le_bytes());
        bytes.extend_from_slice(&row.definition_id.0.to_le_bytes());
        bytes.extend_from_slice(&row.member_feature_id.0.to_le_bytes());
        bytes.extend_from_slice(&row.profile_feature_id.0.to_le_bytes());
        push_projection_bytes(&mut bytes, row.material_key.as_bytes());
        bytes.extend_from_slice(&(row.quantity as u64).to_le_bytes());
        bytes.extend_from_slice(&row.centerline_length_mm.to_bits().to_le_bytes());
        bytes.extend_from_slice(&row.orientation_degrees.to_bits().to_le_bytes());
        for cut in [row.start_cut, row.end_cut] {
            bytes.push(match cut.treatment {
                WeldmentCutTreatment::Square => 0,
                WeldmentCutTreatment::Butt => 1,
                WeldmentCutTreatment::Miter => 2,
            });
            bytes.extend_from_slice(&cut.angle_degrees.to_bits().to_le_bytes());
        }
        bytes.extend_from_slice(&(row.instances.len() as u64).to_le_bytes());
        for instance in &row.instances {
            push_projection_path(&mut bytes, instance);
        }
        push_projection_evidence(&mut bytes, &row.evidence_class);
        push_validation_state(&mut bytes, row.validation_state);
    }
    bytes
}

fn weldment_drawing_bytes(
    rows: &[WeldmentCutListRow],
    cut_list_digest: &str,
    validation_state: ValidationState,
) -> Vec<u8> {
    let mut bytes = WELDMENT_DRAWING_SVG_V1.as_bytes().to_vec();
    push_projection_bytes(&mut bytes, cut_list_digest.as_bytes());
    push_projection_bytes(&mut bytes, &weldment_cut_list_bytes(rows, validation_state));
    bytes
}

fn general_bom_bytes(rows: &[GeneralBomRow], evidence_counts: EvidenceCounts) -> Vec<u8> {
    let mut bytes = GENERAL_BOM_EXPORT_V2.as_bytes().to_vec();
    bytes.extend_from_slice(&(rows.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(evidence_counts.exact as u64).to_le_bytes());
    bytes.extend_from_slice(&(evidence_counts.tolerant as u64).to_le_bytes());
    for row in rows {
        push_projection_bytes(&mut bytes, row.stable_row_id.as_bytes());
        bytes.extend_from_slice(&(row.position as u64).to_le_bytes());
        bytes.extend_from_slice(&row.definition_id.0.to_le_bytes());
        push_general_source(&mut bytes, &row.source);
        push_projection_bytes(&mut bytes, row.item_kind.token().as_bytes());
        push_projection_bytes(&mut bytes, row.material_key.as_bytes());
        bytes.extend_from_slice(&(row.quantity as u64).to_le_bytes());
        push_dimensions(&mut bytes, row.dimensions);
        bytes.extend_from_slice(&(row.instances.len() as u64).to_le_bytes());
        for instance in &row.instances {
            push_projection_path(&mut bytes, instance);
        }
        push_projection_evidence(&mut bytes, &row.evidence_class);
        push_validation_state(&mut bytes, row.validation_state);
    }
    bytes
}

fn general_drawing_bytes(
    drawings: &[GeneralPieceDrawing],
    validation_state: ValidationState,
) -> Vec<u8> {
    let mut bytes = GENERAL_DRAWING_SVG_V3.as_bytes().to_vec();
    push_validation_state(&mut bytes, validation_state);
    bytes.extend_from_slice(&(drawings.len() as u64).to_le_bytes());
    for drawing in drawings {
        push_projection_bytes(&mut bytes, drawing.stable_drawing_id.as_bytes());
        push_projection_bytes(&mut bytes, drawing.bom_row_id.as_bytes());
        bytes.extend_from_slice(&(drawing.position as u64).to_le_bytes());
        bytes.extend_from_slice(&drawing.definition_id.0.to_le_bytes());
        push_general_source(&mut bytes, &drawing.source);
        push_projection_bytes(&mut bytes, drawing.item_kind.token().as_bytes());
        push_projection_bytes(&mut bytes, drawing.material_key.as_bytes());
        bytes.extend_from_slice(&(drawing.quantity as u64).to_le_bytes());
        bytes.extend_from_slice(&(drawing.instances.len() as u64).to_le_bytes());
        for instance in &drawing.instances {
            push_projection_path(&mut bytes, instance);
        }
        push_projection_bytes(&mut bytes, drawing.projection_method.as_bytes());
        bytes.extend_from_slice(&(drawing.views.len() as u64).to_le_bytes());
        for view in &drawing.views {
            push_projection_bytes(&mut bytes, view.stable_view_id.as_bytes());
            push_projection_bytes(&mut bytes, view.name.as_bytes());
            push_projection_bytes(&mut bytes, view.horizontal_axis.as_bytes());
            push_projection_bytes(&mut bytes, view.vertical_axis.as_bytes());
            bytes.extend_from_slice(&view.width_mm.to_bits().to_le_bytes());
            bytes.extend_from_slice(&view.height_mm.to_bits().to_le_bytes());
        }
        bytes.extend_from_slice(&(drawing.dimensions.len() as u64).to_le_bytes());
        for dimension in &drawing.dimensions {
            push_projection_bytes(&mut bytes, dimension.stable_dimension_id.as_bytes());
            push_projection_bytes(&mut bytes, dimension.axis.as_bytes());
            bytes.extend_from_slice(&dimension.value_mm.to_bits().to_le_bytes());
        }
        push_projection_bytes(
            &mut bytes,
            &general_manufacturing_bytes(&drawing.machining_operations, &[], validation_state),
        );
        push_projection_evidence(&mut bytes, &drawing.evidence_class);
    }
    bytes
}

fn general_manufacturing_bytes(
    operations: &[GeneralManufacturingOperation],
    unresolved_sources: &[GeneralBodySource],
    validation_state: ValidationState,
) -> Vec<u8> {
    let mut bytes = GENERAL_MANUFACTURING_EXPORT_V2.as_bytes().to_vec();
    push_validation_state(&mut bytes, validation_state);
    bytes.extend_from_slice(&(operations.len() as u64).to_le_bytes());
    for operation in operations {
        push_projection_bytes(&mut bytes, operation.stable_operation_id.as_bytes());
        bytes.extend_from_slice(&operation.definition_id.0.to_le_bytes());
        bytes.extend_from_slice(&operation.producer_feature_id.0.to_le_bytes());
        bytes.push(match operation.kind {
            GeneralManufacturingKind::Stock => 0,
            GeneralManufacturingKind::ThroughCut => 1,
            GeneralManufacturingKind::ProfileCut => 2,
            GeneralManufacturingKind::CircularDrill => 3,
            GeneralManufacturingKind::BooleanCut => 4,
        });
        bytes.extend_from_slice(&(operation.semantic_inputs.len() as u64).to_le_bytes());
        for input in &operation.semantic_inputs {
            bytes.extend_from_slice(&input.0.to_le_bytes());
        }
        push_projection_bytes(&mut bytes, operation.frame.as_bytes());
        push_dimensions(&mut bytes, operation.bounds);
        push_machining_geometry(&mut bytes, &operation.machining);
        push_general_source(
            &mut bytes,
            &GeneralBodySource::Exact(operation.source.clone()),
        );
    }
    bytes.extend_from_slice(&(unresolved_sources.len() as u64).to_le_bytes());
    for source in unresolved_sources {
        push_general_source(&mut bytes, source);
    }
    bytes
}

fn push_general_source(bytes: &mut Vec<u8>, source: &GeneralBodySource) {
    match source {
        GeneralBodySource::Exact(key) => {
            bytes.push(0);
            bytes.extend_from_slice(&key.document_id.0.to_le_bytes());
            bytes.extend_from_slice(&key.source_revision.to_le_bytes());
            push_projection_bytes(bytes, key.source_digest.as_bytes());
            bytes.extend_from_slice(&key.definition_id.0.to_le_bytes());
            bytes.extend_from_slice(&key.producer_feature_id.0.to_le_bytes());
            push_projection_bytes(bytes, key.canonical_input_digest.as_bytes());
            push_projection_bytes(bytes, key.exact_input_digest.as_bytes());
            push_projection_bytes(bytes, key.evaluator.as_bytes());
            push_projection_bytes(bytes, key.backend.as_bytes());
            push_projection_bytes(bytes, key.tolerance.as_bytes());
            push_projection_bytes(bytes, key.schema.as_bytes());
            push_projection_bytes(bytes, key.result_fingerprint.as_bytes());
        }
        GeneralBodySource::CanonicalMesh {
            definition_id,
            feature_id,
            geometry_digest,
        } => {
            bytes.push(1);
            bytes.extend_from_slice(&definition_id.0.to_le_bytes());
            bytes.extend_from_slice(&feature_id.0.to_le_bytes());
            push_projection_bytes(bytes, geometry_digest.as_bytes());
        }
        GeneralBodySource::CanonicalExtrusion {
            definition_id,
            profile_id,
            extrusion_id,
            geometry_digest,
        } => {
            bytes.push(2);
            bytes.extend_from_slice(&definition_id.0.to_le_bytes());
            bytes.extend_from_slice(&profile_id.0.to_le_bytes());
            bytes.extend_from_slice(&extrusion_id.0.to_le_bytes());
            push_projection_bytes(bytes, geometry_digest.as_bytes());
        }
        GeneralBodySource::CanonicalExactGraph {
            definition_id,
            producer_feature_id,
            graph_digest,
        } => {
            bytes.push(3);
            bytes.extend_from_slice(&definition_id.0.to_le_bytes());
            bytes.extend_from_slice(&producer_feature_id.0.to_le_bytes());
            push_projection_bytes(bytes, graph_digest.as_bytes());
        }
    }
}

fn push_dimensions(bytes: &mut Vec<u8>, dimensions: PieceDimensions) {
    bytes.extend_from_slice(&dimensions.length_mm.to_bits().to_le_bytes());
    bytes.extend_from_slice(&dimensions.width_mm.to_bits().to_le_bytes());
    bytes.extend_from_slice(&dimensions.height_mm.to_bits().to_le_bytes());
}

fn push_validation_state(bytes: &mut Vec<u8>, state: ValidationState) {
    bytes.push(match state {
        ValidationState::Passed => 0,
        ValidationState::Failed => 1,
        ValidationState::NotEvaluated => 2,
        ValidationState::Unavailable => 3,
    });
}

fn evidence_token(evidence: &EvidenceClass) -> &'static str {
    match evidence {
        EvidenceClass::Exact => "Exact",
        EvidenceClass::Tolerant(_) => "Tolerant",
    }
}

fn format_number(value: f64) -> String {
    let formatted = format!("{value:.9}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_stock_geometry(dimensions: PieceDimensions) -> GeneralMachiningGeometry {
        let points = [
            [0.0, 0.0],
            [dimensions.length_mm, 0.0],
            [dimensions.length_mm, dimensions.width_mm],
            [0.0, dimensions.width_mm],
        ];
        GeneralMachiningGeometry::TimberStock {
            frame: identity_machining_frame(),
            cross_section: (0..4)
                .map(|i| GeneralMachiningSegment::Line {
                    start_mm: points[i],
                    end_mm: points[(i + 1) % 4],
                })
                .collect(),
            start_mm: [0.0; 3],
            length_axis: [0.0, 0.0, 1.0],
            length_mm: dimensions.height_mm,
            cross_section_width_mm: dimensions.length_mm,
            cross_section_height_mm: dimensions.width_mm,
        }
    }

    #[test]
    fn btlx_numeric_values_follow_supported_xsd_ranges() {
        assert_eq!(btlx_component_identifiers(1, 0), Some((1, 1)));
        assert_eq!(
            btlx_component_identifiers(i32::MAX as usize, 0),
            Some((i32::MAX, 1))
        );
        assert_eq!(btlx_component_identifiers(i32::MAX as usize + 1, 0), None);
        let last_index = u32::MAX as usize - 1;
        assert_eq!(
            btlx_component_identifiers(1, last_index),
            Some((1, u32::MAX))
        );
        assert_eq!(btlx_component_identifiers(1, last_index + 1), None);
        assert_eq!(
            format_btlx_positive_number(1.0e-9),
            Some("0.000000001".to_owned())
        );
        assert_eq!(format_btlx_positive_number(1.0e-10), None);
        assert_eq!(format_btlx_positive_number(f64::NAN), None);
        assert_eq!(format_btlx_positive_number(f64::INFINITY), None);
    }

    #[test]
    fn btlx_stock_dimensions_require_a_closed_axis_aligned_rectangle() {
        let valid = test_stock_geometry(PieceDimensions {
            length_mm: 100.0,
            width_mm: 50.0,
            height_mm: 1000.0,
        });
        assert_eq!(
            rectangular_timber_stock_dimensions(&valid),
            Some((1000.0, 100.0, 50.0))
        );

        let mut translated = valid.clone();
        let GeneralMachiningGeometry::TimberStock { start_mm, .. } = &mut translated else {
            unreachable!()
        };
        *start_mm = [10.0, 0.0, 0.0];
        assert_eq!(rectangular_timber_stock_dimensions(&translated), None);

        let mut rotated = valid.clone();
        let GeneralMachiningGeometry::TimberStock { frame, .. } = &mut rotated else {
            unreachable!()
        };
        frame.x_axis = [0.0, 1.0, 0.0];
        frame.y_axis = [-1.0, 0.0, 0.0];
        assert_eq!(rectangular_timber_stock_dimensions(&rotated), None);

        let mut open = valid.clone();
        let GeneralMachiningGeometry::TimberStock { cross_section, .. } = &mut open else {
            unreachable!()
        };
        let GeneralMachiningSegment::Line { end_mm, .. } = &mut cross_section[3] else {
            unreachable!()
        };
        *end_mm = [1.0, 0.0];
        assert_eq!(rectangular_timber_stock_dimensions(&open), None);

        let mut diagonal = valid;
        let GeneralMachiningGeometry::TimberStock { cross_section, .. } = &mut diagonal else {
            unreachable!()
        };
        let GeneralMachiningSegment::Line { end_mm, .. } = &mut cross_section[0] else {
            unreachable!()
        };
        *end_mm = [100.0, 1.0];
        assert_eq!(rectangular_timber_stock_dimensions(&diagonal), None);
    }

    #[test]
    fn woodwop_drilling_maps_horizontal_and_top_vertical_axes_and_rejects_unsafe_axes() {
        let beam_stock = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
            length_mm: 100.0,
            width_mm: 50.0,
            height_mm: 1000.0,
        }))
        .unwrap();
        assert_eq!(beam_stock.definition_axes, [2, 0, 1]);
        assert_eq!(beam_stock.dimensions_mm, [1000.0, 100.0, 50.0]);
        let horizontal = GeneralMachiningGeometry::CircularDrill {
            frame: identity_machining_frame(),
            center_mm: [50.0, 25.0],
            diameter_mm: 10.0,
            start_mm: 0.0,
            end_mm: 50.0,
        };
        let horizontal_macro = woodwop_drilling_macro(&horizontal, beam_stock).unwrap();
        assert!(horizontal_macro.starts_with(
            "<103 \\BohrHoriz\\\nXA=\"0\"\nYA=\"50\"\nZA=\"25\"\nBM=\"XP\"\nTI=\"50\"\nDU=\"10\"\n"
        ));

        let panel_stock = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
            length_mm: 600.0,
            width_mm: 400.0,
            height_mm: 19.0,
        }))
        .unwrap();
        assert_eq!(panel_stock.definition_axes, [0, 1, 2]);
        assert_eq!(panel_stock.dimensions_mm, [600.0, 400.0, 19.0]);
        let vertical = GeneralMachiningGeometry::CircularDrill {
            frame: GeneralMachiningFrame {
                origin_mm: [0.0, 0.0, 19.0],
                x_axis: [0.0, 1.0, 0.0],
                y_axis: [1.0, 0.0, 0.0],
                normal: [0.0, 0.0, -1.0],
            },
            center_mm: [25.0, 50.0],
            diameter_mm: 5.0,
            start_mm: 0.0,
            end_mm: 12.0,
        };
        let vertical_macro = woodwop_drilling_macro(&vertical, panel_stock).unwrap();
        assert!(vertical_macro.starts_with(
            "<102 \\BohrVert\\\nXA=\"50\"\nYA=\"25\"\nBM=\"LS\"\nTI=\"12\"\nDU=\"5\"\n"
        ));
        let mut through = vertical.clone();
        let GeneralMachiningGeometry::CircularDrill { end_mm, .. } = &mut through else {
            unreachable!()
        };
        *end_mm = 19.0;
        assert_eq!(woodwop_drilling_macro(&through, panel_stock), None);

        let root_half = 0.5_f64.sqrt();
        let angled = GeneralMachiningGeometry::CircularDrill {
            frame: GeneralMachiningFrame {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, root_half, root_half],
                normal: [0.0, -root_half, root_half],
            },
            center_mm: [25.0, 25.0],
            diameter_mm: 5.0,
            start_mm: 0.0,
            end_mm: 12.0,
        };
        assert_eq!(woodwop_drilling_macro(&angled, panel_stock), None);

        let from_below = GeneralMachiningGeometry::CircularDrill {
            frame: identity_machining_frame(),
            center_mm: [50.0, 25.0],
            diameter_mm: 5.0,
            start_mm: 0.0,
            end_mm: 12.0,
        };
        assert_eq!(woodwop_drilling_macro(&from_below, panel_stock), None);
    }

    #[test]
    fn woodwop_vertical_pocket_requires_top_rectangular_geometry_and_explicit_tool() {
        let stock = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
            length_mm: 600.0,
            width_mm: 400.0,
            height_mm: 19.0,
        }))
        .unwrap();
        let pocket = GeneralMachiningGeometry::ProfileCut {
            frame: GeneralMachiningFrame {
                origin_mm: [0.0, 0.0, 19.0],
                x_axis: [0.0, 1.0, 0.0],
                y_axis: [1.0, 0.0, 0.0],
                normal: [0.0, 0.0, -1.0],
            },
            segments: vec![
                GeneralMachiningSegment::Line {
                    start_mm: [100.0, 200.0],
                    end_mm: [140.0, 200.0],
                },
                GeneralMachiningSegment::Line {
                    start_mm: [140.0, 200.0],
                    end_mm: [140.0, 260.0],
                },
                GeneralMachiningSegment::Line {
                    start_mm: [140.0, 260.0],
                    end_mm: [100.0, 260.0],
                },
                GeneralMachiningSegment::Line {
                    start_mm: [100.0, 260.0],
                    end_mm: [100.0, 200.0],
                },
            ],
            start_mm: 0.0,
            end_mm: 12.0,
        };

        let macro_text = woodwop_vertical_pocket_macro(&pocket, stock, 101).unwrap();
        assert!(macro_text.starts_with(
            "<112 \\Tasche\\\nXA=\"230\"\nYA=\"120\"\nLA=\"60\"\nBR=\"40\"\nRD=\"0\"\nWI=\"0\"\nTI=\"12\"\n"
        ));
        assert!(macro_text.contains("T_=\"101\"\nF_=\"STANDARD\"\n"));
        assert_eq!(woodwop_vertical_pocket_macro(&pocket, stock, 0), None);

        let mut through = pocket.clone();
        let GeneralMachiningGeometry::ProfileCut { end_mm, .. } = &mut through else {
            unreachable!()
        };
        *end_mm = 19.0;
        assert_eq!(woodwop_vertical_pocket_macro(&through, stock, 101), None);

        let mut open = pocket;
        let GeneralMachiningGeometry::ProfileCut { segments, .. } = &mut open else {
            unreachable!()
        };
        let GeneralMachiningSegment::Line { end_mm, .. } = &mut segments[3] else {
            unreachable!()
        };
        *end_mm = [110.0, 200.0];
        assert_eq!(woodwop_vertical_pocket_macro(&open, stock, 101), None);
    }

    #[test]
    fn homag_dowel_macro_and_code128_label_share_the_machine_program_identity() {
        let stock = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
            length_mm: 600.0,
            width_mm: 400.0,
            height_mm: 19.0,
        }))
        .unwrap();
        let hole = DowelHole {
            stable_hole_id: "dowel-7/0/first".to_owned(),
            instance_path: InstancePath::root(OccurrenceId(1)),
            entry_local_mm: [50.0, 20.0, 19.0],
            inward_unit_local: [0.0, 0.0, -1.0],
            diameter_mm: 8.0,
            depth_mm: 16.0,
            shared_center_world_mm: [50.0, 20.0, 19.0],
        };
        let drilling = woodwop_dowel_macro(&hole, stock).unwrap();
        assert!(drilling.starts_with(
            "<102 \\BohrVert\\\nXA=\"50\"\nYA=\"20\"\nBM=\"LS\"\nTI=\"16\"\nDU=\"8\"\n"
        ));
        let mpr = woodwop_mpr_output(stock, &[drilling], HOMAG_BHX_PRODUCTION_PACKAGE_V1).unwrap();
        let mpr = String::from_utf8(mpr).unwrap();
        assert!(mpr.contains("\\ketchup.homag-bhx-production-package.v1\\"));
        assert_eq!(mpr.matches("\\BohrVert\\").count(), 1);

        let label = homag_code128_svg("ABCDEF123456").unwrap();
        let label_again = homag_code128_svg("ABCDEF123456").unwrap();
        assert_eq!(label, label_again);
        let label = String::from_utf8(label).unwrap();
        assert!(label.contains(HOMAG_CODE128_LABEL_V1));
        assert!(label.contains(">ABCDEF123456</text>"));
        assert!(label.contains("aria-label=\"HOMAG program ABCDEF123456\""));
        assert!(homag_code128_svg("SHORT").is_none());
        assert!(homag_code128_svg("12345678901!").is_none());
    }

    #[test]
    fn btlx_drilling_maps_definition_axes_and_rejects_invalid_geometry() {
        let valid = GeneralMachiningGeometry::CircularDrill {
            frame: identity_machining_frame(),
            center_mm: [50.0, 25.0],
            diameter_mm: 10.0,
            start_mm: 10.0,
            end_mm: 60.0,
        };
        let drilling = btlx_drilling(&valid).unwrap();
        assert_eq!(drilling.reference_point_mm, [10.0, 0.0, 0.0]);
        assert_eq!(drilling.x_vector, [0.0, 1.0, 0.0]);
        assert_eq!(drilling.y_vector, [0.0, 0.0, 1.0]);
        assert_eq!(
            (
                drilling.start_x.as_str(),
                drilling.start_y.as_str(),
                drilling.depth.as_str(),
                drilling.diameter.as_str(),
            ),
            ("50", "25", "50", "10")
        );

        let mut zero_depth = valid.clone();
        let GeneralMachiningGeometry::CircularDrill {
            start_mm, end_mm, ..
        } = &mut zero_depth
        else {
            unreachable!()
        };
        *end_mm = *start_mm;
        assert_eq!(btlx_drilling(&zero_depth), None);

        let mut invalid_diameter = valid.clone();
        let GeneralMachiningGeometry::CircularDrill { diameter_mm, .. } = &mut invalid_diameter
        else {
            unreachable!()
        };
        *diameter_mm = 50_000.000_000_001;
        assert_eq!(btlx_drilling(&invalid_diameter), None);

        let mut invalid_center = valid.clone();
        let GeneralMachiningGeometry::CircularDrill { center_mm, .. } = &mut invalid_center else {
            unreachable!()
        };
        *center_mm = [100_000.000_000_001, 25.0];
        assert_eq!(btlx_drilling(&invalid_center), None);

        let mut invalid_frame = valid;
        let GeneralMachiningGeometry::CircularDrill { frame, .. } = &mut invalid_frame else {
            unreachable!()
        };
        frame.x_axis = [2.0, 0.0, 0.0];
        assert_eq!(btlx_drilling(&invalid_frame), None);
    }

    #[test]
    fn btlx_free_contour_requires_a_closed_simple_line_arc_profile() {
        let line = |start_mm, end_mm| GeneralMachiningSegment::Line { start_mm, end_mm };
        let valid = GeneralMachiningGeometry::ProfileCut {
            frame: identity_machining_frame(),
            segments: vec![
                line([0.0, 0.0], [20.0, 0.0]),
                line([20.0, 0.0], [20.0, 10.0]),
                line([20.0, 10.0], [0.0, 10.0]),
                line([0.0, 10.0], [0.0, 0.0]),
            ],
            start_mm: 10.0,
            end_mm: 30.0,
        };
        let contour = btlx_free_contour(&valid).unwrap();
        assert_eq!(contour.reference_point_mm, [10.0, 0.0, 0.0]);
        assert_eq!(contour.tool_position, "left");
        assert_eq!(contour.start_point, ["0".to_owned(), "0".to_owned()]);
        assert_eq!(contour.depth, "20");

        let clockwise = GeneralMachiningGeometry::ProfileCut {
            frame: identity_machining_frame(),
            segments: vec![
                line([0.0, 0.0], [0.0, 10.0]),
                line([0.0, 10.0], [20.0, 10.0]),
                line([20.0, 10.0], [20.0, 0.0]),
                line([20.0, 0.0], [0.0, 0.0]),
            ],
            start_mm: 0.0,
            end_mm: 20.0,
        };
        assert_eq!(
            btlx_free_contour(&clockwise).unwrap().tool_position,
            "right"
        );

        let mut open = valid.clone();
        let GeneralMachiningGeometry::ProfileCut { segments, .. } = &mut open else {
            unreachable!()
        };
        let GeneralMachiningSegment::Line { end_mm, .. } = &mut segments[3] else {
            unreachable!()
        };
        *end_mm = [1.0, 0.0];
        assert_eq!(btlx_free_contour(&open), None);

        let irregular = GeneralMachiningGeometry::ProfileCut {
            frame: identity_machining_frame(),
            segments: vec![
                line([0.0, 0.0], [20.0, 0.0]),
                line([20.0, 0.0], [25.0, 5.0]),
                line([25.0, 5.0], [10.0, 15.0]),
                line([10.0, 15.0], [0.0, 10.0]),
                line([0.0, 10.0], [0.0, 0.0]),
            ],
            start_mm: 0.0,
            end_mm: 20.0,
        };
        assert_eq!(btlx_free_contour(&irregular).unwrap().segments.len(), 5);

        let arc_profile = GeneralMachiningGeometry::ProfileCut {
            frame: identity_machining_frame(),
            segments: vec![
                line([10.0, 10.0], [30.0, 10.0]),
                GeneralMachiningSegment::CircularArc {
                    start_mm: [30.0, 10.0],
                    end_mm: [30.0, 30.0],
                    center_mm: [30.0, 20.0],
                    clockwise: false,
                },
                line([30.0, 30.0], [10.0, 30.0]),
                line([10.0, 30.0], [10.0, 10.0]),
            ],
            start_mm: 0.0,
            end_mm: 18.0,
        };
        assert!(matches!(
            &btlx_free_contour(&arc_profile).unwrap().segments[1],
            BtlxContourSegment::Arc {
                end_point,
                point_on_arc,
            } if end_point == &["30".to_owned(), "30".to_owned()]
                && point_on_arc == &["40".to_owned(), "20".to_owned()]
        ));
        let mut mismatched_arc_radius = arc_profile;
        let GeneralMachiningGeometry::ProfileCut { segments, .. } = &mut mismatched_arc_radius
        else {
            unreachable!()
        };
        let GeneralMachiningSegment::CircularArc { center_mm, .. } = &mut segments[1] else {
            unreachable!()
        };
        *center_mm = [31.0, 19.0];
        assert_eq!(btlx_free_contour(&mismatched_arc_radius), None);

        let self_intersecting = GeneralMachiningGeometry::ProfileCut {
            frame: identity_machining_frame(),
            segments: vec![
                line([0.0, 0.0], [20.0, 20.0]),
                line([20.0, 20.0], [0.0, 20.0]),
                line([0.0, 20.0], [20.0, 0.0]),
                line([20.0, 0.0], [0.0, 0.0]),
            ],
            start_mm: 0.0,
            end_mm: 20.0,
        };
        assert_eq!(btlx_free_contour(&self_intersecting), None);

        let collapsed_after_formatting = GeneralMachiningGeometry::ProfileCut {
            frame: identity_machining_frame(),
            segments: vec![
                line([0.0, 0.0], [1.0e-10, 0.0]),
                line([1.0e-10, 0.0], [1.0e-10, 10.0]),
                line([1.0e-10, 10.0], [0.0, 10.0]),
                line([0.0, 10.0], [0.0, 0.0]),
            ],
            start_mm: 0.0,
            end_mm: 20.0,
        };
        assert_eq!(btlx_free_contour(&collapsed_after_formatting), None);

        let mut excessive_depth = valid;
        let GeneralMachiningGeometry::ProfileCut { end_mm, .. } = &mut excessive_depth else {
            unreachable!()
        };
        *end_mm = 100_011.0;
        assert_eq!(btlx_free_contour(&excessive_depth), None);
    }
}
