use crate::document::{
    CanonicalCommand, CanonicalError, CanonicalOverride, CommandBatch, Dimension, DocumentStore,
    EvaluatorNodeKind, NodeId, OverrideParameterSpec, PortSpec, RuleOutput, SlotPath,
    SlotResolution, SlotSegment,
};
use crate::fabrication::{
    DimensionChain, DimensionDatumRef, DimensionSegment, FabricationProjectionEnvelope,
    FullBomProjection, FullBomRow, PieceDimensionSheet, PieceDimensions, ProjectionStatus,
};
use crate::graph::DerivedIdentity;
use crate::prismatic::{
    Aabb, CanonicalJoint, JointId, JointValidationOutcome, PrismaticError, TolerancePolicy,
    validate_joint_overlap,
};
use crate::validation::{
    BuiltinPrismaticValidator, EvidenceCounts, HostNeutralValidator, PrismaticJointCase,
    ValidationExecution, ValidationInvocation, ValidationReport, ValidationState,
    beam_validation_policy, prismatic_input_bytes,
};
use std::collections::BTreeSet;
use std::fmt;

pub const BEAM_TOTAL_MM: f64 = 7260.0;
pub const BEAM_WIDTH_MM: f64 = 200.0;
pub const BEAM_HEIGHT_MM: f64 = 420.0;
pub const GROOVE_WIDTH_MM: f64 = 160.0;
pub const GROOVE_DEPTH_MM: f64 = 20.0;
pub const BASELINE_ZONE1_GAP_MM: f64 = 415.0;
pub const ZONE2_GAP_MM: f64 = 408.0;
pub const TERMINAL_GAP_MM: f64 = 400.0;
pub const END_RESIDUAL_MM: f64 = 200.0;
pub const ZONE1_GAP_NODE: NodeId = NodeId(4001);
pub const BEAM_RULE_NODE: NodeId = NodeId(4002);
pub const UNRELATED_NODE: NodeId = NodeId(4099);
pub const GROOVE_OVERRIDE_ID: u64 = 4001;
const OUTPUT_PORT: &str = "beam_pieces";
const OVERRIDE_PARAMETER: &str = "groove_depth";
const OVERRIDE_GROOVE_NUMBER: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroovePosition {
    pub number: usize,
    pub start_mm: f64,
    pub end_mm: f64,
    pub centre_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PieceKind {
    BeamBody,
    CrossingJointProxy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DerivedProxyPiece {
    pub identity: DerivedIdentity,
    pub kind: PieceKind,
    pub bounds: Aabb,
    pub length_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BomRow {
    pub stable_group_key: &'static str,
    pub kind: PieceKind,
    pub quantity: usize,
    pub length_mm: f64,
    pub piece_identities: Vec<DerivedIdentity>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupedBom {
    pub generated_for_revision: u64,
    pub rows: Vec<BomRow>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeamValidationVerdict {
    Green,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BeamSlice {
    pub revision_id: u64,
    pub positions: Vec<GroovePosition>,
    pub pieces: Vec<DerivedProxyPiece>,
    pub bom: GroupedBom,
    pub full_bom: FullBomProjection,
    pub dimension_sheet: PieceDimensionSheet,
    pub joint_outcomes: Vec<JointValidationOutcome>,
    pub validation_report: ValidationReport,
    pub validation: BeamValidationVerdict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeamChangeSummary {
    pub revision_id: u64,
    pub recomputed_nodes: BTreeSet<NodeId>,
    pub recomputed_piece_identities: BTreeSet<DerivedIdentity>,
    pub bom_regenerated: bool,
    pub dimensions_regenerated: bool,
    pub validator_ran: bool,
}

pub struct BeamWorkspace {
    document: DocumentStore,
    slice: BeamSlice,
    last_change: Option<BeamChangeSummary>,
}

impl BeamWorkspace {
    pub fn load() -> Result<Self, BeamError> {
        let positions = positions(BASELINE_ZONE1_GAP_MM, 7, 5)?;
        let body = body_identity()?;
        let mut commands = vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: ZONE1_GAP_NODE,
                name: "Beam A zone 1 gap".to_owned(),
                dimension: Dimension::new("415", BASELINE_ZONE1_GAP_MM)?,
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: BEAM_RULE_NODE,
                name: "Beam A M4a-E rule".to_owned(),
                expression: "$4001".to_owned(),
                input_ports: vec![PortSpec::number("zone1_gap")?],
                output_ports: vec![PortSpec::number(OUTPUT_PORT)?],
                outputs: rule_outputs(7, 5)?,
                override_parameters: vec![OverrideParameterSpec::replace(OVERRIDE_PARAMETER)?],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: UNRELATED_NODE,
                name: "Unrelated fixture input".to_owned(),
                dimension: Dimension::new("1", 1.0)?,
                dependencies: vec![],
            },
            CanonicalCommand::UpsertOverride(CanonicalOverride::new(
                GROOVE_OVERRIDE_ID,
                groove_identity("zone1", OVERRIDE_GROOVE_NUMBER)?,
                OVERRIDE_PARAMETER,
                GROOVE_DEPTH_MM,
                SlotResolution::Resolved,
            )?),
        ];
        commands.extend(joint_commands(&body, &positions)?);
        let mut document = DocumentStore::new();
        document.apply_batch(&CommandBatch::new(commands))?;
        document.discard_history_before_current();
        let slice = derive_slice(&document)?;
        Ok(Self {
            document,
            slice,
            last_change: None,
        })
    }

    #[must_use]
    pub const fn slice(&self) -> &BeamSlice {
        &self.slice
    }

    #[must_use]
    pub fn snapshot(&self) -> crate::document::Snapshot {
        self.document.current()
    }

    #[must_use]
    pub const fn last_change(&self) -> Option<&BeamChangeSummary> {
        self.last_change.as_ref()
    }

    pub fn set_zone1_gap_mm(&mut self, gap_mm: f64) -> Result<&BeamChangeSummary, BeamError> {
        if !gap_mm.is_finite() || gap_mm <= 0.0 {
            return Err(BeamError::InvalidFixture);
        }
        let zone1_count = zone_counts(&self.document)?.0;
        let positions = positions(gap_mm, zone1_count, 5)?;
        let body = body_identity()?;
        let mut commands = vec![CanonicalCommand::SetEvaluatorDimension {
            id: ZONE1_GAP_NODE,
            dimension: Dimension::new(format_number(gap_mm), gap_mm)?,
        }];
        commands.extend(joint_commands(&body, &positions)?);
        self.apply_change(commands)
    }

    pub fn set_zone1_count(&mut self, count: usize) -> Result<&BeamChangeSummary, BeamError> {
        if count > 7 {
            return Err(BeamError::InvalidFixture);
        }
        let gap = zone1_gap(&self.document)?;
        let positions = positions(gap, count, 5)?;
        let body = body_identity()?;
        let mut commands = vec![CanonicalCommand::SetRuleOutputs {
            id: BEAM_RULE_NODE,
            outputs: rule_outputs(count, 5)?,
        }];
        for number in (count + 1)..=7 {
            commands.push(CanonicalCommand::DeleteJoint {
                id: JointId(number as u64),
            });
        }
        commands.extend(joint_commands(&body, &positions)?);
        self.apply_change(commands)
    }

    pub fn duplicate_override_key(&mut self) -> Result<&BeamChangeSummary, BeamError> {
        let mut outputs = rule_outputs(7, 5)?;
        let mut children = outputs[1].children().to_vec();
        children.push(RuleOutput::new(
            segment(&format!("groove-g{OVERRIDE_GROOVE_NUMBER:02}"))?,
            vec![],
        )?);
        outputs[1] = RuleOutput::new(segment("zone1")?, children)?;
        self.apply_change(vec![CanonicalCommand::SetRuleOutputs {
            id: BEAM_RULE_NODE,
            outputs,
        }])
    }

    fn apply_change(
        &mut self,
        commands: Vec<CanonicalCommand>,
    ) -> Result<&BeamChangeSummary, BeamError> {
        let before = self.slice.clone();
        let revision = self.document.apply_batch(&CommandBatch::new(commands))?;
        let after = derive_slice(&self.document)?;
        let changed = changed_pieces(&before, &after);
        self.last_change = Some(BeamChangeSummary {
            revision_id: revision.id(),
            recomputed_nodes: revision.recomputed_nodes().clone(),
            recomputed_piece_identities: changed,
            bom_regenerated: before.full_bom.envelope.source_revision
                != after.full_bom.envelope.source_revision,
            dimensions_regenerated: before.dimension_sheet.envelope.source_revision
                != after.dimension_sheet.envelope.source_revision,
            validator_ran: true,
        });
        self.slice = after;
        Ok(self
            .last_change
            .as_ref()
            .expect("change summary was assigned"))
    }
}

fn derive_slice(document: &DocumentStore) -> Result<BeamSlice, BeamError> {
    let snapshot = document.current();
    let (zone1_count, zone2_count) = zone_counts(document)?;
    let positions = positions(zone1_gap(document)?, zone1_count, zone2_count)?;
    let body_id = body_identity()?;
    let body_bounds = Aabb::bounded_volume(
        [0.0, 0.0, 0.0],
        [BEAM_TOTAL_MM, BEAM_WIDTH_MM, BEAM_HEIGHT_MM],
    )?;
    let mut pieces = vec![DerivedProxyPiece {
        identity: body_id.clone(),
        kind: PieceKind::BeamBody,
        bounds: body_bounds,
        length_mm: BEAM_TOTAL_MM,
    }];
    for position in &positions {
        let zone = if position.number <= 7 {
            "zone1"
        } else {
            "zone2"
        };
        pieces.push(DerivedProxyPiece {
            identity: groove_identity(zone, position.number)?,
            kind: PieceKind::CrossingJointProxy,
            bounds: Aabb::bounded_volume(
                [position.start_mm, 0.0, BEAM_HEIGHT_MM - GROOVE_DEPTH_MM],
                [
                    position.end_mm,
                    BEAM_WIDTH_MM,
                    BEAM_HEIGHT_MM + GROOVE_DEPTH_MM,
                ],
            )?,
            length_mm: GROOVE_WIDTH_MM,
        });
    }
    let tolerance = TolerancePolicy::default();
    let cases = pieces
        .iter()
        .skip(1)
        .map(|proxy| PrismaticJointCase {
            left_identity: body_id.clone(),
            right_identity: proxy.identity.clone(),
            left_evidence_class: crate::validation::EvidenceClass::Exact,
            right_evidence_class: crate::validation::EvidenceClass::Exact,
            left_bounds: body_bounds,
            right_bounds: proxy.bounds,
            declared_joint: snapshot
                .joints()
                .find(|joint| joint.connects(&body_id, &proxy.identity))
                .cloned(),
        })
        .collect::<Vec<_>>();
    let outcomes = cases
        .iter()
        .map(|case| {
            validate_joint_overlap(
                case.left_bounds,
                case.right_bounds,
                case.declared_joint.as_ref(),
                tolerance,
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let validator = BuiltinPrismaticValidator::new(tolerance);
    let policy = beam_validation_policy();
    let input = prismatic_input_bytes(&cases, tolerance);
    let invocation = ValidationInvocation::bind(
        &snapshot,
        validator.descriptor(),
        &policy,
        pieces.iter().map(|piece| piece.identity.clone()).collect(),
        &input,
    );
    let validation_report = validator.invoke(ValidationExecution {
        snapshot: &snapshot,
        invocation,
        policy: &policy,
        input: &cases,
    });
    let validation = if validation_report.state == ValidationState::Passed {
        BeamValidationVerdict::Green
    } else {
        BeamValidationVerdict::Error
    };
    let body_identities = vec![body_id.clone()];
    let crossing_identities = pieces
        .iter()
        .skip(1)
        .map(|piece| piece.identity.clone())
        .collect::<Vec<_>>();
    let bom = GroupedBom {
        generated_for_revision: snapshot.revision_id(),
        rows: vec![
            BomRow {
                stable_group_key: "beam-a-body",
                kind: PieceKind::BeamBody,
                quantity: 1,
                length_mm: BEAM_TOTAL_MM,
                piece_identities: body_identities,
            },
            BomRow {
                stable_group_key: "beam-a-crossing-proxy",
                kind: PieceKind::CrossingJointProxy,
                quantity: crossing_identities.len(),
                length_mm: GROOVE_WIDTH_MM,
                piece_identities: crossing_identities,
            },
        ],
    };
    let full_bom = full_bom_projection(&snapshot, &pieces, &validation_report);
    let dimension_sheet = dimension_sheet_projection(&snapshot, &body_id, &positions)?;
    Ok(BeamSlice {
        revision_id: snapshot.revision_id(),
        positions,
        pieces,
        bom,
        full_bom,
        dimension_sheet,
        joint_outcomes: outcomes,
        validation_report,
        validation,
    })
}

fn full_bom_projection(
    snapshot: &crate::document::Snapshot,
    pieces: &[DerivedProxyPiece],
    validation_report: &ValidationReport,
) -> FullBomProjection {
    let validation_state = validation_report.state;
    let body_identities = pieces
        .iter()
        .filter(|piece| piece.kind == PieceKind::BeamBody)
        .map(|piece| piece.identity.clone())
        .collect::<Vec<_>>();
    let crossing_identities = pieces
        .iter()
        .filter(|piece| piece.kind == PieceKind::CrossingJointProxy)
        .map(|piece| piece.identity.clone())
        .collect::<Vec<_>>();
    let rows = vec![
        FullBomRow {
            stable_row_id: "beam-a/body".to_owned(),
            definition_key: "beam-a/member-definition".to_owned(),
            piece_kind: "timber-beam-body".to_owned(),
            material_key: "ketchup.material.timber.unspecified.v1".to_owned(),
            quantity: body_identities.len(),
            dimensions: PieceDimensions {
                length_mm: BEAM_TOTAL_MM,
                width_mm: BEAM_WIDTH_MM,
                height_mm: BEAM_HEIGHT_MM,
            },
            piece_identities: body_identities,
            validation_state,
        },
        FullBomRow {
            stable_row_id: "beam-a/crossing-members".to_owned(),
            definition_key: "beam-a/crossing-member-proxy-definition".to_owned(),
            piece_kind: "crossing-member-proxy".to_owned(),
            material_key: "ketchup.material.fixture-proxy.v1".to_owned(),
            quantity: crossing_identities.len(),
            dimensions: PieceDimensions {
                length_mm: GROOVE_WIDTH_MM,
                width_mm: BEAM_WIDTH_MM,
                height_mm: GROOVE_DEPTH_MM * 2.0,
            },
            piece_identities: crossing_identities,
            validation_state,
        },
    ];
    let result_bytes = full_bom_bytes(&rows, validation_report.evidence_counts);
    FullBomProjection {
        envelope: FabricationProjectionEnvelope::new(
            snapshot,
            &result_bytes,
            if validation_state == ValidationState::Passed {
                ProjectionStatus::Complete
            } else {
                ProjectionStatus::Incomplete
            },
        ),
        evidence_counts: validation_report.evidence_counts,
        rows,
    }
}

fn dimension_sheet_projection(
    snapshot: &crate::document::Snapshot,
    body: &DerivedIdentity,
    positions: &[GroovePosition],
) -> Result<PieceDimensionSheet, BeamError> {
    let mut spacing_segments = Vec::new();
    for pair in positions.windows(2) {
        let left = &pair[0];
        let right = &pair[1];
        let left_zone = if left.number <= 7 { "zone1" } else { "zone2" };
        let right_zone = if right.number <= 7 { "zone1" } else { "zone2" };
        spacing_segments.push(DimensionSegment {
            stable_segment_id: format!("beam-a/gap/g{:02}-g{:02}", left.number, right.number),
            from: DimensionDatumRef {
                piece: groove_identity(left_zone, left.number)?,
                datum: "end".to_owned(),
            },
            to: DimensionDatumRef {
                piece: groove_identity(right_zone, right.number)?,
                datum: "start".to_owned(),
            },
            value_mm: right.start_mm - left.end_mm,
        });
    }
    if let Some(last) = positions.last() {
        let zone = if last.number <= 7 { "zone1" } else { "zone2" };
        spacing_segments.push(DimensionSegment {
            stable_segment_id: "beam-a/gap/g12-terminal".to_owned(),
            from: DimensionDatumRef {
                piece: groove_identity(zone, last.number)?,
                datum: "end".to_owned(),
            },
            to: DimensionDatumRef {
                piece: body.clone(),
                datum: "terminal-residual-start".to_owned(),
            },
            value_mm: TERMINAL_GAP_MM,
        });
    }
    let grouped_labels = grouped_dimension_labels(&spacing_segments);
    let body_datum = |datum: &str| DimensionDatumRef {
        piece: body.clone(),
        datum: datum.to_owned(),
    };
    let chains = vec![
        DimensionChain {
            stable_chain_id: "beam-a/remainder-distribution".to_owned(),
            axis: "x".to_owned(),
            segments: spacing_segments,
            grouped_labels,
        },
        DimensionChain {
            stable_chain_id: "beam-a/overall-length".to_owned(),
            axis: "x".to_owned(),
            segments: vec![DimensionSegment {
                stable_segment_id: "beam-a/overall-length/value".to_owned(),
                from: body_datum("origin"),
                to: body_datum("end"),
                value_mm: BEAM_TOTAL_MM,
            }],
            grouped_labels: vec![format_number(BEAM_TOTAL_MM)],
        },
        DimensionChain {
            stable_chain_id: "beam-a/cross-section-width".to_owned(),
            axis: "y".to_owned(),
            segments: vec![DimensionSegment {
                stable_segment_id: "beam-a/cross-section-width/value".to_owned(),
                from: body_datum("side-west"),
                to: body_datum("side-east"),
                value_mm: BEAM_WIDTH_MM,
            }],
            grouped_labels: vec![format_number(BEAM_WIDTH_MM)],
        },
        DimensionChain {
            stable_chain_id: "beam-a/cross-section-height".to_owned(),
            axis: "z".to_owned(),
            segments: vec![DimensionSegment {
                stable_segment_id: "beam-a/cross-section-height/value".to_owned(),
                from: body_datum("bottom"),
                to: body_datum("top"),
                value_mm: BEAM_HEIGHT_MM,
            }],
            grouped_labels: vec![format_number(BEAM_HEIGHT_MM)],
        },
    ];
    let stable_sheet_id = "beam-a/fabrication-sheet";
    let named_view = "beam-a-longitudinal-and-cross-section";
    let result_bytes = dimension_sheet_bytes(stable_sheet_id, body, named_view, &chains);
    let status = if snapshot
        .overrides()
        .all(|override_value| override_value.health == SlotResolution::Resolved)
    {
        ProjectionStatus::Complete
    } else {
        ProjectionStatus::Incomplete
    };
    Ok(PieceDimensionSheet {
        envelope: FabricationProjectionEnvelope::new(snapshot, &result_bytes, status),
        stable_sheet_id: stable_sheet_id.to_owned(),
        piece: body.clone(),
        named_view: named_view.to_owned(),
        chains,
    })
}

fn full_bom_bytes(rows: &[FullBomRow], evidence_counts: EvidenceCounts) -> Vec<u8> {
    let mut output = b"ketchup.full-bom.v1".to_vec();
    output.extend_from_slice(&(evidence_counts.exact as u64).to_le_bytes());
    output.extend_from_slice(&(evidence_counts.tolerant as u64).to_le_bytes());
    output.extend_from_slice(&(rows.len() as u64).to_le_bytes());
    for row in rows {
        push_text(&mut output, &row.stable_row_id);
        push_text(&mut output, &row.definition_key);
        push_text(&mut output, &row.piece_kind);
        push_text(&mut output, &row.material_key);
        output.extend_from_slice(&(row.quantity as u64).to_le_bytes());
        push_number(&mut output, row.dimensions.length_mm);
        push_number(&mut output, row.dimensions.width_mm);
        push_number(&mut output, row.dimensions.height_mm);
        output.extend_from_slice(&(row.piece_identities.len() as u64).to_le_bytes());
        for identity in &row.piece_identities {
            push_identity(&mut output, identity);
        }
        output.push(match row.validation_state {
            ValidationState::Passed => 0,
            ValidationState::Failed => 1,
            ValidationState::NotEvaluated => 2,
            ValidationState::Unavailable => 3,
        });
    }
    output
}

fn dimension_sheet_bytes(
    stable_sheet_id: &str,
    piece: &DerivedIdentity,
    named_view: &str,
    chains: &[DimensionChain],
) -> Vec<u8> {
    let mut output = b"ketchup.dimension-sheet.v1".to_vec();
    push_text(&mut output, stable_sheet_id);
    push_identity(&mut output, piece);
    push_text(&mut output, named_view);
    output.extend_from_slice(&(chains.len() as u64).to_le_bytes());
    for chain in chains {
        push_text(&mut output, &chain.stable_chain_id);
        push_text(&mut output, &chain.axis);
        output.extend_from_slice(&(chain.segments.len() as u64).to_le_bytes());
        for segment in &chain.segments {
            push_text(&mut output, &segment.stable_segment_id);
            push_identity(&mut output, &segment.from.piece);
            push_text(&mut output, &segment.from.datum);
            push_identity(&mut output, &segment.to.piece);
            push_text(&mut output, &segment.to.datum);
            push_number(&mut output, segment.value_mm);
        }
        output.extend_from_slice(&(chain.grouped_labels.len() as u64).to_le_bytes());
        for label in &chain.grouped_labels {
            push_text(&mut output, label);
        }
    }
    output
}

fn push_identity(output: &mut Vec<u8>, identity: &DerivedIdentity) {
    output.extend_from_slice(&identity.root_rule_node_id.0.to_le_bytes());
    output.extend_from_slice(&(identity.slot_path.segments().len() as u64).to_le_bytes());
    for segment in identity.slot_path.segments() {
        output.extend_from_slice(&segment.producer_rule_id.0.to_le_bytes());
        push_text(output, &segment.output_port);
        push_text(output, &segment.semantic_key);
    }
}

fn push_text(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn push_number(output: &mut Vec<u8>, value: f64) {
    output.extend_from_slice(&value.to_bits().to_le_bytes());
}

fn grouped_dimension_labels(segments: &[DimensionSegment]) -> Vec<String> {
    let mut labels = Vec::new();
    let mut index = 0;
    while index < segments.len() {
        let value = segments[index].value_mm;
        let mut count = 1;
        while index + count < segments.len()
            && segments[index + count].value_mm.to_bits() == value.to_bits()
        {
            count += 1;
        }
        if count == 1 {
            labels.push(format_number(value));
        } else {
            labels.push(format!("{} × {count}", format_number(value)));
        }
        index += count;
    }
    labels
}

fn zone1_gap(document: &DocumentStore) -> Result<f64, BeamError> {
    document
        .current()
        .evaluator_node(ZONE1_GAP_NODE)
        .and_then(|node| node.dimension())
        .map(|value| value.millimetres())
        .ok_or(BeamError::InvalidFixture)
}

fn zone_counts(document: &DocumentStore) -> Result<(usize, usize), BeamError> {
    let snapshot = document.current();
    let node = snapshot
        .evaluator_node(BEAM_RULE_NODE)
        .ok_or(BeamError::InvalidFixture)?;
    let EvaluatorNodeKind::Rule { outputs, .. } = node.kind() else {
        return Err(BeamError::InvalidFixture);
    };
    let zone = |key: &str| {
        outputs
            .iter()
            .find(|output| output.segment().semantic_key == key)
            .map(|output| output.children().len())
            .unwrap_or(0)
    };
    Ok((zone("zone1"), zone("zone2")))
}

fn positions(
    zone1_gap: f64,
    zone1_count: usize,
    zone2_count: usize,
) -> Result<Vec<GroovePosition>, BeamError> {
    let mut result = Vec::new();
    let zone1_spacing = GROOVE_WIDTH_MM + zone1_gap;
    for index in 0..zone1_count {
        let start = 210.0 + index as f64 * zone1_spacing;
        result.push(position(index + 1, start));
    }
    let zone1_last_centre = result
        .last()
        .map_or(290.0 - zone1_spacing, |item| item.centre_mm);
    let zone2_spacing = GROOVE_WIDTH_MM + ZONE2_GAP_MM;
    for index in 0..zone2_count {
        let centre = zone1_last_centre + (index + 1) as f64 * zone2_spacing;
        result.push(position(8 + index, centre - GROOVE_WIDTH_MM * 0.5));
    }
    if result.iter().any(|item| item.end_mm > BEAM_TOTAL_MM) {
        return Err(BeamError::InvalidFixture);
    }
    Ok(result)
}

fn position(number: usize, start_mm: f64) -> GroovePosition {
    GroovePosition {
        number,
        start_mm,
        end_mm: start_mm + GROOVE_WIDTH_MM,
        centre_mm: start_mm + GROOVE_WIDTH_MM * 0.5,
    }
}

fn rule_outputs(zone1_count: usize, zone2_count: usize) -> Result<Vec<RuleOutput>, BeamError> {
    let zone = |key: &str, numbers: Vec<usize>| -> Result<RuleOutput, BeamError> {
        let children = numbers
            .into_iter()
            .map(|number| {
                RuleOutput::new(segment(&format!("groove-g{number:02}"))?, vec![])
                    .map_err(BeamError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RuleOutput::new(segment(key)?, children)?)
    };
    Ok(vec![
        RuleOutput::new(segment("beam-body")?, vec![])?,
        zone("zone1", (1..=zone1_count).collect())?,
        zone("zone2", (8..(8 + zone2_count)).collect())?,
    ])
}

fn segment(key: &str) -> Result<SlotSegment, BeamError> {
    Ok(SlotSegment::new(BEAM_RULE_NODE, OUTPUT_PORT, key)?)
}

fn body_identity() -> Result<DerivedIdentity, BeamError> {
    Ok(DerivedIdentity::new(
        BEAM_RULE_NODE,
        SlotPath::new(vec![segment("beam-body")?])?,
    )?)
}

fn groove_identity(zone: &str, number: usize) -> Result<DerivedIdentity, BeamError> {
    Ok(DerivedIdentity::new(
        BEAM_RULE_NODE,
        SlotPath::new(vec![
            segment(zone)?,
            segment(&format!("groove-g{number:02}"))?,
        ])?,
    )?)
}

fn joint_commands(
    body: &DerivedIdentity,
    positions: &[GroovePosition],
) -> Result<Vec<CanonicalCommand>, BeamError> {
    positions
        .iter()
        .map(|position| {
            let zone = if position.number <= 7 {
                "zone1"
            } else {
                "zone2"
            };
            let volume = Aabb::bounded_volume(
                [position.start_mm, 0.0, BEAM_HEIGHT_MM - GROOVE_DEPTH_MM],
                [position.end_mm, BEAM_WIDTH_MM, BEAM_HEIGHT_MM],
            )?;
            Ok(CanonicalCommand::UpsertJoint(CanonicalJoint::new(
                JointId(position.number as u64),
                body.clone(),
                groove_identity(zone, position.number)?,
                volume,
            )?))
        })
        .collect()
}

fn changed_pieces(before: &BeamSlice, after: &BeamSlice) -> BTreeSet<DerivedIdentity> {
    after
        .pieces
        .iter()
        .filter(|piece| {
            before
                .pieces
                .iter()
                .find(|old| old.identity == piece.identity)
                .is_none_or(|old| old != *piece)
        })
        .map(|piece| piece.identity.clone())
        .collect()
}

fn format_number(value: f64) -> String {
    let value = format!("{value:.6}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[derive(Debug)]
pub enum BeamError {
    Canonical(CanonicalError),
    Prismatic(PrismaticError),
    InvalidFixture,
}

impl fmt::Display for BeamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canonical(error) => error.fmt(formatter),
            Self::Prismatic(error) => error.fmt(formatter),
            Self::InvalidFixture => formatter.write_str("Beam A fixture is invalid"),
        }
    }
}

impl std::error::Error for BeamError {}
impl From<CanonicalError> for BeamError {
    fn from(value: CanonicalError) -> Self {
        Self::Canonical(value)
    }
}
impl From<PrismaticError> for BeamError {
    fn from(value: PrismaticError) -> Self {
        Self::Prismatic(value)
    }
}
impl From<crate::graph::GraphError> for BeamError {
    fn from(value: crate::graph::GraphError) -> Self {
        Self::Canonical(CanonicalError::Graph(value))
    }
}
