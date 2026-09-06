use crate::document::{
    BooleanOperation, DefinitionId, DocumentId, FeatureId, FeatureKind, InstancePath,
    InstancePathStep, OccurrenceId, Snapshot, Transform,
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
use crate::prismatic::TolerancePolicy;
use crate::validation::{
    EvidenceClass, EvidenceCounts, PermittedErrorDirection, TolerantEvidence, ValidationReport,
    ValidationState,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

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

pub const GENERAL_FABRICATION_EVALUATOR_V3: &str = "ketchup.general-fabrication-evaluator.v3";
pub const FABRICATION_ROLE_DIMENSION_V1: &str = "ketchup.fabrication-role.v1";
pub const TIMBER_MEMBER_ROLE_V1: &str = "fabrication.timber-member.v1";
pub const TIMBER_MATERIAL_V1: &str = "ketchup.material.timber.unspecified.v1";
pub const GENERAL_BOM_EXPORT_V1: &str = "ketchup.general-bom-export.v1";
pub const GENERAL_DRAWING_SVG_V1: &str = "ketchup.general-drawing-svg.v1";
pub const GENERAL_MANUFACTURING_EXPORT_V2: &str = "ketchup.general-manufacturing-export.v2";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralFabricationError {
    ValidationBindingMismatch,
    FabricationRoleDimensionMissing,
    FabricationRoleDimensionAmbiguous,
    TimberMemberRoleMissing,
    TimberMemberRoleAmbiguous,
    UnsupportedOrUnavailableGeometry,
    InvalidGeometry,
    NoSupportedGeometry,
    ExportBlocked,
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
            Self::UnsupportedOrUnavailableGeometry => formatter.write_str(
                "a visible geometry-bearing occurrence has unsupported or unavailable evidence",
            ),
            Self::InvalidGeometry => {
                formatter.write_str("accepted fabrication geometry has invalid local bounds")
            }
            Self::NoSupportedGeometry => {
                formatter.write_str("the document contains no supported visible body geometry")
            }
            Self::ExportBlocked => formatter.write_str(
                "fabrication export is incomplete, stale, invalid, or lacks manufacturing semantics",
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

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralBomRow {
    pub stable_row_id: String,
    pub definition_id: DefinitionId,
    pub source: GeneralBodySource,
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
    pub definition_id: DefinitionId,
    pub source: GeneralBodySource,
    pub projection_method: &'static str,
    pub views: Vec<GeneralDrawingView>,
    pub dimensions: Vec<GeneralDimensionCallout>,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneralMachiningFrame {
    pub origin_mm: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub normal: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralFabricationProjection {
    pub bom: GeneralBomProjection,
    pub drawings: GeneralDrawingProjection,
    pub manufacturing: GeneralManufacturingProjection,
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
            "{GENERAL_BOM_EXPORT_V1}\ndocument_id={}\nsource_revision={}\nsource_digest={}\nresult_digest={}\n",
            self.bom.envelope.document_id.0,
            self.bom.envelope.source_revision,
            self.bom.envelope.source_digest,
            self.bom.envelope.result_digest
        );
        for row in &self.bom.rows {
            output.push_str(&format!(
                "row={};definition={};quantity={};length_mm={};width_mm={};height_mm={};material={};evidence={}\n",
                row.stable_row_id,
                row.definition_id.0,
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
        let result_bytes =
            general_drawing_bytes(&self.drawings.drawings, self.drawings.validation_state);
        if self.drawings.envelope.status != ProjectionStatus::Complete
            || !general_envelope_is_current(&self.drawings.envelope, snapshot)
            || self.drawings.envelope.result_digest != sha256_hex(&result_bytes)
            || self.drawings.validation_state != ValidationState::Passed
            || self.drawings.drawings.is_empty()
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let sheet_width = self
            .drawings
            .drawings
            .iter()
            .flat_map(|drawing| drawing.views.iter().map(|view| view.width_mm))
            .fold(0.0_f64, f64::max)
            .max(100.0)
            + 80.0;
        let sheet_height = self
            .drawings
            .drawings
            .iter()
            .map(|drawing| {
                drawing
                    .views
                    .iter()
                    .map(|view| view.height_mm + 55.0)
                    .sum::<f64>()
                    + 35.0
            })
            .sum::<f64>()
            + 40.0;
        let mut svg = format!(
            "<!-- {GENERAL_DRAWING_SVG_V1} {} -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\">\n",
            self.drawings.envelope.result_digest,
            format_number(sheet_width),
            format_number(sheet_height)
        );
        let mut y = 25.0;
        for drawing in &self.drawings.drawings {
            svg.push_str(&format!(
                "<g id=\"{}\" fill=\"none\" stroke=\"black\">\n",
                drawing.stable_drawing_id
            ));
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
            svg.push_str("</g>\n");
            y += 35.0;
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
}

fn timber_member_occurrences(
    snapshot: &Snapshot,
) -> Result<BTreeSet<OccurrenceId>, GeneralFabricationError> {
    let dimensions = snapshot
        .classification_dimensions()
        .filter(|dimension| dimension.name() == FABRICATION_ROLE_DIMENSION_V1)
        .collect::<Vec<_>>();
    let [dimension] = dimensions.as_slice() else {
        return Err(if dimensions.is_empty() {
            GeneralFabricationError::FabricationRoleDimensionMissing
        } else {
            GeneralFabricationError::FabricationRoleDimensionAmbiguous
        });
    };
    let categories = dimension
        .categories()
        .filter(|category| category.name() == TIMBER_MEMBER_ROLE_V1)
        .collect::<Vec<_>>();
    let [timber_category] = categories.as_slice() else {
        return Err(if categories.is_empty() {
            GeneralFabricationError::TimberMemberRoleMissing
        } else {
            GeneralFabricationError::TimberMemberRoleAmbiguous
        });
    };
    Ok(snapshot
        .occurrences()
        .filter(|occurrence| {
            snapshot.occurrence_classification(occurrence.id(), dimension.id())
                == Some(timber_category.id())
        })
        .map(|occurrence| occurrence.id())
        .collect())
}

pub fn project_general_fabrication(
    snapshot: &Snapshot,
    registry: &ExactResultRegistry,
    validation_cases: &[GeneralClearanceCase],
    validation_report: &ValidationReport,
    tolerance: TolerancePolicy,
) -> Result<GeneralFabricationProjection, GeneralFabricationError> {
    let timber_members = timber_member_occurrences(snapshot)?;
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
    let mut accepted = Vec::new();
    for occurrence in snapshot.scene_query().into_iter().filter(|occurrence| {
        occurrence.visible && timber_members.contains(&occurrence.occurrence_id)
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
        if !covered.contains(&(
            participant.instance_path().clone(),
            participant.source().clone(),
        )) {
            return Err(GeneralFabricationError::ValidationBindingMismatch);
        }
        let dimensions = local_dimensions(snapshot, registry, participant.source())?;
        accepted.push((participant, dimensions));
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
    for (participant, _) in &accepted {
        evidence_counts.record(participant.evidence_class());
    }
    let validation_state = validation_report.state;
    let general_status = if validation_state == ValidationState::Passed {
        ProjectionStatus::Complete
    } else {
        ProjectionStatus::Incomplete
    };
    let mut grouped =
        BTreeMap::<(GeneralBodySource, [u64; 3]), Vec<&GeneralBodyParticipant>>::new();
    for (participant, dimensions) in &accepted {
        grouped
            .entry((
                participant.source().clone(),
                [
                    dimensions.length_mm.to_bits(),
                    dimensions.width_mm.to_bits(),
                    dimensions.height_mm.to_bits(),
                ],
            ))
            .or_default()
            .push(participant);
    }
    let mut rows = Vec::new();
    for ((source, dimension_bits), participants) in grouped {
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
                GENERAL_FABRICATION_EVALUATOR_V3,
                PermittedErrorDirection::BidirectionalBounded,
            )
            .expect("the fabrication tolerance and method identity are valid"),
        );
        rows.push(GeneralBomRow {
            stable_row_id: format!(
                "definition-{}/source-{}",
                definition_id.0,
                source_digest_token(&source)
            ),
            definition_id,
            source,
            material_key: TIMBER_MATERIAL_V1.to_owned(),
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
            GENERAL_FABRICATION_EVALUATOR_V3,
        ),
        evidence_counts,
        rows,
    };

    let drawings = bom
        .rows
        .iter()
        .map(general_piece_drawing)
        .collect::<Vec<_>>();
    let drawing_bytes = general_drawing_bytes(&drawings, validation_state);
    let drawings = GeneralDrawingProjection {
        envelope: FabricationProjectionEnvelope::new_with_evaluator(
            snapshot,
            &drawing_bytes,
            general_status,
            GENERAL_FABRICATION_EVALUATOR_V3,
        ),
        validation_state,
        drawings,
    };

    let mut operations = Vec::new();
    let mut unresolved_sources = Vec::new();
    for row in &bom.rows {
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
            GENERAL_FABRICATION_EVALUATOR_V3,
        ),
        validation_state,
        operations,
        unresolved_sources,
    };
    Ok(GeneralFabricationProjection {
        bom,
        drawings,
        manufacturing,
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
        machining: legacy_stock_geometry(row.dimensions),
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

fn rectangle_machining_segments(
    minimum: [f64; 2],
    maximum: [f64; 2],
) -> Vec<GeneralMachiningSegment> {
    let points = [
        minimum,
        [maximum[0], minimum[1]],
        maximum,
        [minimum[0], maximum[1]],
    ];
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(start_mm, end_mm)| GeneralMachiningSegment::Line {
            start_mm: *start_mm,
            end_mm: *end_mm,
        })
        .collect()
}

fn legacy_stock_geometry(dimensions: PieceDimensions) -> GeneralMachiningGeometry {
    GeneralMachiningGeometry::TimberStock {
        frame: identity_machining_frame(),
        cross_section: rectangle_machining_segments(
            [0.0, 0.0],
            [dimensions.length_mm, dimensions.width_mm],
        ),
        start_mm: [0.0, 0.0, 0.0],
        length_axis: [0.0, 0.0, 1.0],
        length_mm: dimensions.height_mm,
        cross_section_width_mm: dimensions.length_mm,
        cross_section_height_mm: dimensions.width_mm,
    }
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

fn general_envelope_is_current(
    envelope: &FabricationProjectionEnvelope,
    snapshot: &Snapshot,
) -> bool {
    envelope.projection_schema == FABRICATION_PROJECTION_V1
        && envelope.evaluator_id == GENERAL_FABRICATION_EVALUATOR_V3
        && envelope.is_current(snapshot)
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

fn general_piece_drawing(row: &GeneralBomRow) -> GeneralPieceDrawing {
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
        definition_id: row.definition_id,
        source: row.source.clone(),
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

fn general_bom_bytes(rows: &[GeneralBomRow], evidence_counts: EvidenceCounts) -> Vec<u8> {
    let mut bytes = GENERAL_BOM_EXPORT_V1.as_bytes().to_vec();
    bytes.extend_from_slice(&(rows.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(evidence_counts.exact as u64).to_le_bytes());
    bytes.extend_from_slice(&(evidence_counts.tolerant as u64).to_le_bytes());
    for row in rows {
        push_projection_bytes(&mut bytes, row.stable_row_id.as_bytes());
        bytes.extend_from_slice(&row.definition_id.0.to_le_bytes());
        push_general_source(&mut bytes, &row.source);
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
    let mut bytes = GENERAL_DRAWING_SVG_V1.as_bytes().to_vec();
    push_validation_state(&mut bytes, validation_state);
    bytes.extend_from_slice(&(drawings.len() as u64).to_le_bytes());
    for drawing in drawings {
        push_projection_bytes(&mut bytes, drawing.stable_drawing_id.as_bytes());
        bytes.extend_from_slice(&drawing.definition_id.0.to_le_bytes());
        push_general_source(&mut bytes, &drawing.source);
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
