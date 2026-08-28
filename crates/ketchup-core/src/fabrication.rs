use crate::document::{
    BooleanOperation, DefinitionId, DocumentId, FeatureId, FeatureKind, InstancePath,
    InstancePathStep, Snapshot, Transform,
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

pub const GENERAL_FABRICATION_EVALUATOR_V1: &str = "ketchup.general-fabrication-evaluator.v1";
pub const GENERAL_BOM_EXPORT_V1: &str = "ketchup.general-bom-export.v1";
pub const GENERAL_DRAWING_SVG_V1: &str = "ketchup.general-drawing-svg.v1";
pub const GENERAL_MANUFACTURING_EXPORT_V1: &str = "ketchup.general-manufacturing-export.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralFabricationError {
    ValidationBindingMismatch,
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
    BooleanCut,
}

impl GeneralManufacturingKind {
    const fn token(self) -> &'static str {
        match self {
            Self::Stock => "stock",
            Self::ThroughCut => "through-cut",
            Self::BooleanCut => "boolean-cut",
        }
    }
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
            "{GENERAL_MANUFACTURING_EXPORT_V1}\ndocument_id={}\nsource_revision={}\nsource_digest={}\nresult_digest={}\n",
            self.manufacturing.envelope.document_id.0,
            self.manufacturing.envelope.source_revision,
            self.manufacturing.envelope.source_digest,
            self.manufacturing.envelope.result_digest
        );
        for operation in &self.manufacturing.operations {
            output.push_str(&format!(
                "operation={};definition={};producer={};kind={};frame={};inputs={};length_mm={};width_mm={};height_mm={};result_fingerprint={}\n",
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
                operation.source.result_fingerprint
            ));
        }
        Ok(output.into_bytes())
    }
}

pub fn project_general_fabrication(
    snapshot: &Snapshot,
    registry: &ExactResultRegistry,
    validation_cases: &[GeneralClearanceCase],
    validation_report: &ValidationReport,
    tolerance: TolerancePolicy,
) -> Result<GeneralFabricationProjection, GeneralFabricationError> {
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
    for occurrence in snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
    {
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
                GENERAL_FABRICATION_EVALUATOR_V1,
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
            material_key: "ketchup.material.unspecified.v1".to_owned(),
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
            GENERAL_FABRICATION_EVALUATOR_V1,
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
            GENERAL_FABRICATION_EVALUATOR_V1,
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
                let definition = snapshot
                    .definition(row.definition_id)
                    .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
                let has_union = definition.feature_ids().iter().any(|feature_id| {
                    matches!(
                        snapshot.feature(*feature_id).map(|feature| feature.kind()),
                        Some(FeatureKind::Boolean {
                            operation: BooleanOperation::Union,
                            ..
                        })
                    )
                });
                if !matches!(package.as_ref(), ExactBodyPackage::Rectangle(_)) || has_union {
                    unresolved_sources.push(row.source.clone());
                    continue;
                }
                operations.push(GeneralManufacturingOperation {
                    stable_operation_id: format!("definition-{}/stock", row.definition_id.0),
                    definition_id: row.definition_id,
                    producer_feature_id: source.producer_feature_id,
                    kind: GeneralManufacturingKind::Stock,
                    semantic_inputs: Vec::new(),
                    frame: "definition-local",
                    bounds: row.dimensions,
                    source: source.clone(),
                });
                for feature_id in definition.feature_ids() {
                    let feature = snapshot
                        .feature(*feature_id)
                        .ok_or(GeneralFabricationError::UnsupportedOrUnavailableGeometry)?;
                    let (kind, semantic_inputs) = match feature.kind() {
                        FeatureKind::ThroughCut { target, profile } => (
                            GeneralManufacturingKind::ThroughCut,
                            vec![*target, *profile],
                        ),
                        FeatureKind::Boolean {
                            operation: BooleanOperation::Cut,
                            target,
                            tool,
                        } => (GeneralManufacturingKind::BooleanCut, vec![*target, *tool]),
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
                        source: source.clone(),
                    });
                }
            }
            GeneralBodySource::CanonicalMesh { .. }
            | GeneralBodySource::CanonicalExtrusion { .. } => {
                unresolved_sources.push(row.source.clone());
            }
        }
    }
    operations.sort_by(|left, right| left.stable_operation_id.cmp(&right.stable_operation_id));
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
            GENERAL_FABRICATION_EVALUATOR_V1,
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

fn general_envelope_is_current(
    envelope: &FabricationProjectionEnvelope,
    snapshot: &Snapshot,
) -> bool {
    envelope.projection_schema == FABRICATION_PROJECTION_V1
        && envelope.evaluator_id == GENERAL_FABRICATION_EVALUATOR_V1
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
        | GeneralBodySource::CanonicalExtrusion { definition_id, .. } => *definition_id,
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
    let mut bytes = GENERAL_MANUFACTURING_EXPORT_V1.as_bytes().to_vec();
    push_validation_state(&mut bytes, validation_state);
    bytes.extend_from_slice(&(operations.len() as u64).to_le_bytes());
    for operation in operations {
        push_projection_bytes(&mut bytes, operation.stable_operation_id.as_bytes());
        bytes.extend_from_slice(&operation.definition_id.0.to_le_bytes());
        bytes.extend_from_slice(&operation.producer_feature_id.0.to_le_bytes());
        bytes.push(match operation.kind {
            GeneralManufacturingKind::Stock => 0,
            GeneralManufacturingKind::ThroughCut => 1,
            GeneralManufacturingKind::BooleanCut => 2,
        });
        bytes.extend_from_slice(&(operation.semantic_inputs.len() as u64).to_le_bytes());
        for input in &operation.semantic_inputs {
            bytes.extend_from_slice(&input.0.to_le_bytes());
        }
        push_projection_bytes(&mut bytes, operation.frame.as_bytes());
        push_dimensions(&mut bytes, operation.bounds);
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
