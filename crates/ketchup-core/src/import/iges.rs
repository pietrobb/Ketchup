use std::collections::BTreeSet;
use std::fmt;

use crate::document::{
    BodyKind, CanonicalCommand, CommandBatch, DefinitionId, FeatureId, FeatureKind,
    IMPORTED_EXACT_BODY_SCHEMA_V3, ImportedExactBodySpec, OccurrenceId, Snapshot, Transform,
};
use crate::graph::sha256_bytes;

use super::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportLengthUnit, ImportOutputRef,
    ImportReceipt, ImportUnitAuthority, ImportUnitDecision,
};

pub const IGES_PARSER_ID: &str = "ketchup-occt-iges";
pub const IGES_PARSER_VERSION: &str = "1";
pub const IGES_XDE_PARSER_VERSION: &str = "2";
pub const MAX_IGES_SOURCE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct IgesImportEvidence {
    pub source_sha256: [u8; 32],
    pub source_byte_len: u64,
    pub source_unit: ImportLengthUnit,
    pub result_fingerprint: String,
    pub body_kind: BodyKind,
    pub solid_count: u32,
    pub topology_counts: [u32; 5],
    pub area_mm2: f64,
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IgesXdePartEvidence {
    pub index: u32,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
    pub exact: super::StepImportEvidence,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IgesXdeNodeEvidence {
    pub id: u32,
    pub part_index: u32,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
    pub transform: Transform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IgesXdeImportEvidence {
    pub source_sha256: [u8; 32],
    pub source_byte_len: u64,
    pub parts: Vec<IgesXdePartEvidence>,
    pub nodes: Vec<IgesXdeNodeEvidence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IgesImportPlanError {
    Empty,
    SourceTooLarge,
    InvalidSourceIdentity,
    InvalidWorkerEvidence,
    IdSpaceExhausted,
}

impl fmt::Display for IgesImportPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "IGES source is empty",
            Self::SourceTooLarge => "IGES source exceeds the bounded 32 MiB envelope",
            Self::InvalidSourceIdentity => "IGES source name or provenance is invalid",
            Self::InvalidWorkerEvidence => "IGES worker evidence is incomplete or invalid",
            Self::IdSpaceExhausted => "canonical import ID space is exhausted",
        })
    }
}

impl std::error::Error for IgesImportPlanError {}

pub fn plan_iges_import(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
    evidence: &IgesImportEvidence,
) -> Result<CommandBatch, IgesImportPlanError> {
    if source.is_empty() {
        return Err(IgesImportPlanError::Empty);
    }
    if source.len() as u64 > MAX_IGES_SOURCE_BYTES {
        return Err(IgesImportPlanError::SourceTooLarge);
    }
    let exact = super::StepImportEvidence {
        source_unit: evidence.source_unit,
        result_fingerprint: evidence.result_fingerprint.clone(),
        body_kind: evidence.body_kind,
        solid_count: evidence.solid_count,
        topology_counts: evidence.topology_counts,
        area_mm2: evidence.area_mm2,
        volume_mm3: evidence.volume_mm3,
        bounds_mm: evidence.bounds_mm,
        backend: evidence.backend.clone(),
        tolerance: evidence.tolerance.clone(),
    };
    if evidence.source_sha256 != sha256_bytes(source)
        || evidence.source_byte_len != source.len() as u64
        || !super::exact_body_evidence_valid(&exact)
    {
        return Err(IgesImportPlanError::InvalidWorkerEvidence);
    }
    let next_id = |ids: Vec<u64>| {
        ids.into_iter()
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .filter(|id| *id != 0)
            .ok_or(IgesImportPlanError::IdSpaceExhausted)
    };
    let import_id = snapshot
        .next_import_id()
        .map_err(|_| IgesImportPlanError::IdSpaceExhausted)?;
    let definition_id = DefinitionId(next_id(
        snapshot.definitions().map(|item| item.id().0).collect(),
    )?);
    let feature_id = FeatureId(next_id(
        snapshot.features().map(|item| item.id().0).collect(),
    )?);
    let occurrence_id = OccurrenceId(next_id(
        snapshot.occurrences().map(|item| item.id().0).collect(),
    )?);
    let outputs = vec![
        ImportOutputRef::Definition(definition_id),
        ImportOutputRef::Feature(feature_id),
        ImportOutputRef::Occurrence(occurrence_id),
    ];
    let diagnostics = [
        (ImportDiagnosticSeverity::Info, "iges_exact_brep_preserved"),
        (
            ImportDiagnosticSeverity::Warning,
            "iges_color_metadata_unavailable",
        ),
        (
            ImportDiagnosticSeverity::Warning,
            "iges_hierarchy_flattened",
        ),
        (
            ImportDiagnosticSeverity::Warning,
            "iges_name_metadata_unavailable",
        ),
        (
            ImportDiagnosticSeverity::Warning,
            "iges_parametric_reconstruction_unavailable",
        ),
    ]
    .into_iter()
    .map(|(severity, code)| ImportDiagnostic::new(severity, code, None, 1))
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| IgesImportPlanError::InvalidWorkerEvidence)?;
    let units = ImportUnitDecision::new(evidence.source_unit, ImportUnitAuthority::FileDeclared);
    let receipt = ImportReceipt::from_source_bytes(
        import_id,
        ImportFormat::Iges,
        source,
        source_name,
        units,
        IGES_PARSER_ID,
        IGES_PARSER_VERSION,
        diagnostics,
        outputs,
    )
    .map_err(|_| IgesImportPlanError::InvalidSourceIdentity)?;
    let display_name = source_name
        .strip_suffix(".iges")
        .or_else(|| source_name.strip_suffix(".igs"))
        .or_else(|| source_name.strip_suffix(".IGES"))
        .or_else(|| source_name.strip_suffix(".IGS"))
        .filter(|name| !name.is_empty())
        .unwrap_or(source_name)
        .to_owned();
    Ok(CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: display_name.clone(),
        },
        CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: "Imported IGES exact body".to_owned(),
            kind: FeatureKind::ImportedExactBody(ImportedExactBodySpec {
                schema: IMPORTED_EXACT_BODY_SCHEMA_V3.to_owned(),
                import_id,
                source_sha256: sha256_bytes(source),
                source_byte_len: source.len() as u64,
                source_part_index: None,
                result_fingerprint: evidence.result_fingerprint.clone(),
                body_kind: evidence.body_kind,
                solid_count: evidence.solid_count,
                topology_counts: Some(evidence.topology_counts),
                area_mm2: evidence.area_mm2,
                volume_mm3: evidence.volume_mm3,
                bounds_mm: evidence.bounds_mm,
                backend: evidence.backend.clone(),
                tolerance: evidence.tolerance.clone(),
            }),
        },
        CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: display_name,
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::RecordImport(receipt),
    ]))
}

fn iges_exact_evidence_valid(evidence: &super::StepImportEvidence) -> bool {
    super::exact_body_evidence_valid(evidence)
}

pub fn plan_iges_xde_import(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
    evidence: &IgesXdeImportEvidence,
) -> Result<CommandBatch, IgesImportPlanError> {
    if source.is_empty() {
        return Err(IgesImportPlanError::Empty);
    }
    if source.len() as u64 > MAX_IGES_SOURCE_BYTES {
        return Err(IgesImportPlanError::SourceTooLarge);
    }
    if evidence.source_sha256 != sha256_bytes(source)
        || evidence.source_byte_len != source.len() as u64
        || evidence.parts.is_empty()
        || evidence.parts.len() > 1_024
        || evidence.nodes.len() != evidence.parts.len()
    {
        return Err(IgesImportPlanError::InvalidWorkerEvidence);
    }
    let source_unit = evidence.parts[0].exact.source_unit;
    let valid_text = |value: &str| {
        !value.is_empty() && value.len() <= 1_024 && !value.chars().any(char::is_control)
    };
    if evidence.parts.iter().enumerate().any(|(index, part)| {
        part.index as usize != index
            || !valid_text(&part.name)
            || part.exact.source_unit != source_unit
            || !iges_exact_evidence_valid(&part.exact)
    }) {
        return Err(IgesImportPlanError::InvalidWorkerEvidence);
    }
    let mut used_parts = BTreeSet::new();
    if evidence.nodes.iter().enumerate().any(|(index, node)| {
        node.id as usize != index
            || node.part_index as usize >= evidence.parts.len()
            || !used_parts.insert(node.part_index)
            || !valid_text(&node.name)
            || node.transform != Transform::identity()
    }) || used_parts.len() != evidence.parts.len()
    {
        return Err(IgesImportPlanError::InvalidWorkerEvidence);
    }

    let next_range = |maximum: u64, count: usize| {
        let start = maximum
            .checked_add(1)
            .filter(|value| *value != 0)
            .ok_or(IgesImportPlanError::IdSpaceExhausted)?;
        start
            .checked_add(count as u64 - 1)
            .ok_or(IgesImportPlanError::IdSpaceExhausted)?;
        Ok(start)
    };
    let definition_start = next_range(
        snapshot
            .definitions()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0),
        evidence.parts.len(),
    )?;
    let feature_start = next_range(
        snapshot
            .features()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0),
        evidence.parts.len(),
    )?;
    let occurrence_start = next_range(
        snapshot
            .occurrences()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0),
        evidence.nodes.len(),
    )?;
    let import_id = snapshot
        .next_import_id()
        .map_err(|_| IgesImportPlanError::IdSpaceExhausted)?;

    let definitions = (0..evidence.parts.len())
        .map(|index| DefinitionId(definition_start + index as u64))
        .collect::<Vec<_>>();
    let mut commands = Vec::new();
    let mut outputs = Vec::new();
    for (index, part) in evidence.parts.iter().enumerate() {
        let definition_id = definitions[index];
        let feature_id = FeatureId(feature_start + index as u64);
        commands.push(CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: part.name.clone(),
        });
        commands.push(CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: "Imported IGES XDE exact root".to_owned(),
            kind: FeatureKind::ImportedExactBody(ImportedExactBodySpec {
                schema: IMPORTED_EXACT_BODY_SCHEMA_V3.to_owned(),
                import_id,
                source_sha256: evidence.source_sha256,
                source_byte_len: evidence.source_byte_len,
                source_part_index: Some(part.index),
                result_fingerprint: part.exact.result_fingerprint.clone(),
                body_kind: part.exact.body_kind,
                solid_count: part.exact.solid_count,
                topology_counts: Some(part.exact.topology_counts),
                area_mm2: part.exact.area_mm2,
                volume_mm3: part.exact.volume_mm3,
                bounds_mm: part.exact.bounds_mm,
                backend: part.exact.backend.clone(),
                tolerance: part.exact.tolerance.clone(),
            }),
        });
        outputs.push(ImportOutputRef::Definition(definition_id));
        outputs.push(ImportOutputRef::Feature(feature_id));
    }
    for (index, node) in evidence.nodes.iter().enumerate() {
        let occurrence_id = OccurrenceId(occurrence_start + index as u64);
        commands.push(CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id: definitions[node.part_index as usize],
            name: node.name.clone(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        });
        let color = node
            .color
            .or(evidence.parts[node.part_index as usize].color);
        if color.is_some() {
            commands.push(CanonicalCommand::SetOccurrenceColor {
                id: occurrence_id,
                color,
            });
        }
        outputs.push(ImportOutputRef::Occurrence(occurrence_id));
    }
    outputs.sort_unstable();

    let missing_names = evidence
        .parts
        .iter()
        .filter(|part| !part.name_from_source)
        .count()
        + evidence
            .nodes
            .iter()
            .filter(|node| !node.name_from_source)
            .count();
    let colored_roots = evidence
        .nodes
        .iter()
        .filter(|node| node.color.is_some())
        .count();
    let mut diagnostics = Vec::new();
    let mut add = |severity, code, count| {
        diagnostics.push(
            ImportDiagnostic::new(severity, code, None, count)
                .map_err(|_| IgesImportPlanError::InvalidWorkerEvidence)?,
        );
        Ok::<(), IgesImportPlanError>(())
    };
    add(
        ImportDiagnosticSeverity::Info,
        "iges_exact_brep_roots_preserved",
        evidence.parts.len() as u32,
    )?;
    if missing_names == 0 {
        add(
            ImportDiagnosticSeverity::Info,
            "iges_name_metadata_preserved",
            evidence.nodes.len() as u32,
        )?;
    } else {
        add(
            ImportDiagnosticSeverity::Warning,
            "iges_name_metadata_unavailable",
            missing_names as u32,
        )?;
    }
    if colored_roots == 0 {
        add(
            ImportDiagnosticSeverity::Warning,
            "iges_color_metadata_unavailable",
            1,
        )?;
    } else {
        add(
            ImportDiagnosticSeverity::Info,
            "iges_color_metadata_preserved",
            colored_roots as u32,
        )?;
    }
    add(
        ImportDiagnosticSeverity::Warning,
        "iges_hierarchy_unavailable_flat_roots_only",
        evidence.nodes.len() as u32,
    )?;
    add(
        ImportDiagnosticSeverity::Warning,
        "iges_shared_definitions_unavailable_roots_duplicated",
        evidence.parts.len() as u32,
    )?;
    add(
        ImportDiagnosticSeverity::Warning,
        "iges_local_transforms_baked_into_exact_geometry",
        evidence.nodes.len() as u32,
    )?;
    add(
        ImportDiagnosticSeverity::Warning,
        "iges_parametric_reconstruction_unavailable",
        evidence.parts.len() as u32,
    )?;
    diagnostics.sort_unstable();
    let receipt = ImportReceipt::from_source_bytes(
        import_id,
        ImportFormat::Iges,
        source,
        source_name,
        ImportUnitDecision::new(source_unit, ImportUnitAuthority::FileDeclared),
        IGES_PARSER_ID,
        IGES_XDE_PARSER_VERSION,
        diagnostics,
        outputs,
    )
    .map_err(|_| IgesImportPlanError::InvalidSourceIdentity)?;
    commands.push(CanonicalCommand::RecordImport(receipt));
    Ok(CommandBatch::new(commands))
}
