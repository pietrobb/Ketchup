use std::fmt;

use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, FeatureKind,
    IMPORTED_EXACT_BODY_SCHEMA_V1, ImportedExactBodySpec, OccurrenceId, Snapshot, Transform,
};
use crate::graph::sha256_bytes;

use super::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportLengthUnit, ImportOutputRef,
    ImportReceipt, ImportUnitAuthority, ImportUnitDecision,
};

pub const IGES_PARSER_ID: &str = "ketchup-occt-iges";
pub const IGES_PARSER_VERSION: &str = "1";
pub const MAX_IGES_SOURCE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct IgesImportEvidence {
    pub source_sha256: [u8; 32],
    pub source_byte_len: u64,
    pub source_unit: ImportLengthUnit,
    pub result_fingerprint: String,
    pub solid_count: u32,
    pub topology_counts: [u32; 5],
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
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
    let bounds_valid = evidence
        .bounds_mm
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        && (0..3).all(|axis| evidence.bounds_mm[0][axis] <= evidence.bounds_mm[1][axis]);
    if evidence.source_sha256 != sha256_bytes(source)
        || evidence.source_byte_len != source.len() as u64
        || evidence.result_fingerprint.is_empty()
        || evidence.solid_count == 0
        || evidence.solid_count > 1_024
        || evidence.topology_counts.contains(&0)
        || evidence.topology_counts[4] != evidence.solid_count
        || evidence.topology_counts[..3]
            .iter()
            .map(|count| u64::from(*count))
            .sum::<u64>()
            > crate::topology::MAX_GENERATED_TOPOLOGICAL_REFERENCES
        || !evidence.volume_mm3.is_finite()
        || evidence.volume_mm3 <= 0.0
        || !bounds_valid
        || evidence.backend.is_empty()
        || evidence.tolerance.is_empty()
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
                schema: IMPORTED_EXACT_BODY_SCHEMA_V1.to_owned(),
                import_id,
                source_sha256: sha256_bytes(source),
                source_byte_len: source.len() as u64,
                result_fingerprint: evidence.result_fingerprint.clone(),
                solid_count: evidence.solid_count,
                topology_counts: Some(evidence.topology_counts),
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
