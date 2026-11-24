//! Parts a program reads from STEP files (`imported()`, `purchased()`): the
//! host reads each file once after the evaluation ([`read_imports`]), and the
//! planner places its exact body as the part's occurrence ([`append_imported`]),
//! so the document stays the program's.

use crate::RuleProgramApplyError;
use crate::diagnostics::{AssistantRejection, assistant_canonical_rejection};
use ketchup_model::document::{
    CanonicalCommand, CanonicalError, DefinitionId, DocumentStore, FeatureId, FeatureKind,
    IMPORTED_EXACT_BODY_SCHEMA_V3, ImportedExactBodySpec, OccurrenceId, Snapshot, Transform,
};
use ketchup_model::import::{
    ImportFormat, ImportId, ImportOutputRef, ImportReceipt, ImportUnitAuthority,
    ImportUnitDecision, STEP_PARSER_ID, STEP_PARSER_VERSION, STEP_XDE_PARSER_VERSION,
    StepImportEvidence,
};
use ketchup_program::ProgramModel;
use ketchup_program::imported::ImportedBody;
use ketchup_program::model::Part;
use ketchup_rejection::{Rejection, RejectionPhase};
use std::collections::BTreeMap;

/// Where the host reads a program's STEP files from.
pub trait ImportFiles {
    /// The body of `file` (as the program names it, relative to the folder
    /// of the program's file), the whole file or its assembly part `part`;
    /// `Ok(None)` when this host has no folder to read it from.
    ///
    /// # Errors
    /// Why the file could not be read: missing, outside the folder, not a
    /// valid STEP body; its reason says it, its cause keeps the error behind.
    fn read(&mut self, file: &str, part: Option<&str>) -> Result<Option<ImportedBody>, Rejection>;
}

/// A host without the program's folder: only what the document already holds.
pub struct NoFiles;

impl ImportFiles for NoFiles {
    fn read(
        &mut self,
        _file: &str,
        _part: Option<&str>,
    ) -> Result<Option<ImportedBody>, Rejection> {
        Ok(None)
    }
}

/// Why a program's file cannot be read, said by `reason`.
#[must_use]
pub fn unreadable(reason: impl Into<String>) -> Rejection {
    Rejection::new(IMPORT_FAILED, RejectionPhase::Io).reason(reason)
}

/// Records in every part of `model` read from a file what `files` read of
/// it, each file and part once; a file this host cannot read is taken from
/// the body the document already places for the part, or for the file.
///
/// # Errors
/// A file could not be read, or nothing here holds it.
pub fn read_imports(
    model: &mut ProgramModel,
    snapshot: &Snapshot,
    files: &mut dyn ImportFiles,
) -> Result<(), RuleProgramApplyError> {
    let mut read = BTreeMap::<(String, Option<String>), ImportedBody>::new();
    for part in model.imported_parts_mut() {
        let Some(import) = part.imported() else {
            continue;
        };
        if import.read.is_some() {
            continue;
        }
        let key = (import.file.clone(), import.part.clone());
        let body = match read.get(&key) {
            Some(body) => body.clone(),
            None => {
                let body = match files.read(&key.0, key.1.as_deref()) {
                    Ok(Some(body)) => body,
                    Ok(None) => from_document(snapshot, &part.name, &key.0, key.1.as_deref())
                        .ok_or_else(|| failed(part, &key.0, &unreadable("this Kečup has no folder of the program's file to read it from; apply the program from its file")))?,
                    Err(rejection) => return Err(failed(part, &key.0, &rejection)),
                };
                read.insert(key, body.clone());
                body
            }
        };
        ketchup_program::imported::ProgramImport::record(part, body);
    }
    Ok(())
}

/// The code of a program whose bought part's file cannot be read.
pub const IMPORT_FAILED: &str = "program_import_failed";

fn failed(part: &Part, file: &str, rejection: &Rejection) -> RuleProgramApplyError {
    let mut why = rejection.reason_text().to_owned();
    for cause in rejection.causes() {
        why.push_str(" (");
        why.push_str(&cause);
        why.push(')');
    }
    RuleProgramApplyError::Program {
        code: IMPORT_FAILED.to_owned(),
        message: format!("{:?}: cannot read {file:?}: {why}", part.name),
        location: None,
    }
}

/// The file's own name, the receipt's name of it.
fn file_name(file: &str) -> &str {
    file.rsplit(['/', '\\']).next().unwrap_or(file)
}

/// What the document holds of `file`: the body it places for the part
/// `name`, or another body read from a file of that name.
fn from_document(
    snapshot: &Snapshot,
    name: &str,
    file: &str,
    part: Option<&str>,
) -> Option<ImportedBody> {
    let bodies = snapshot.definitions().filter_map(|definition| {
        let [feature] = definition.feature_ids() else {
            return None;
        };
        let FeatureKind::ImportedExactBody(spec) = snapshot.feature(*feature)?.kind() else {
            return None;
        };
        let receipt = snapshot.import_receipt(spec.import_id)?;
        (receipt.format() == ImportFormat::Step
            && receipt.source_name() == file_name(file)
            && spec.source_part_index.is_some() == part.is_some())
        .then(|| (definition.id(), body_of(spec, receipt)))
    });
    let mut by_file = None;
    for (definition, body) in bodies {
        let placed_for_part = snapshot.occurrences().any(|occurrence| {
            occurrence.definition_id() == definition && occurrence.name() == name
        });
        if placed_for_part {
            return Some(body);
        }
        // Another part of an assembly file is another body.
        if part.is_none() {
            by_file.get_or_insert(body);
        }
    }
    by_file
}

fn body_of(spec: &ImportedExactBodySpec, receipt: &ImportReceipt) -> ImportedBody {
    ImportedBody {
        file_name: receipt.source_name().to_owned(),
        part_index: spec.source_part_index,
        evidence: StepImportEvidence {
            source_sha256: spec.source_sha256,
            source_byte_len: spec.source_byte_len,
            source_unit: receipt.units().source_unit(),
            result_fingerprint: spec.result_fingerprint.clone(),
            body_kind: spec.body_kind,
            solid_count: spec.solid_count,
            topology_counts: spec.topology_counts.unwrap_or([0; 5]),
            area_mm2: spec.area_mm2,
            volume_mm3: spec.volume_mm3,
            bounds_mm: spec.bounds_mm,
            backend: spec.backend.clone(),
            tolerance: spec.tolerance.clone(),
        },
        placement: spec.source_placement,
    }
}

/// The name of the feature holding `part`'s body: the part's name, its file's
/// and the assembly part it is ([`placed_part_name`] reads it back).
fn feature_name(part: &Part, body: &ImportedBody) -> String {
    let named = part.imported().and_then(|import| import.part.as_deref());
    match (body.part_index, named) {
        (Some(_), Some(named)) => format!(
            "{}{named}",
            part_feature_prefix(&part.name, &body.file_name)
        ),
        _ => format!("{} from {}", part.name, body.file_name),
    }
}

fn part_feature_prefix(definition: &str, file_name: &str) -> String {
    format!("{definition} from {file_name}, part ")
}

/// The assembly part (`part=`) a program placed as `feature`, the definition
/// `definition` of a body read from `file_name`; none for a whole file's.
#[must_use]
pub fn placed_part_name<'a>(
    feature: &'a str,
    definition: &str,
    file_name: &str,
) -> Option<&'a str> {
    feature.strip_prefix(&part_feature_prefix(definition, file_name))
}

fn parser_version(body: &ImportedBody) -> &'static str {
    if body.part_index.is_some() {
        STEP_XDE_PARSER_VERSION
    } else {
        STEP_PARSER_VERSION
    }
}

/// Places every part of `parts` read from a file: a definition holding the
/// file's exact body, an occurrence where the program puts it, and one
/// import receipt per file the document does not hold yet.
///
/// # Errors
/// A part's file was not read, or the document's ID space is used up.
pub(crate) fn append_imported(
    document: &DocumentStore,
    parts: &[&Part],
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), AssistantRejection> {
    if parts.is_empty() {
        return Ok(());
    }
    let snapshot = document.current();
    let created = |pick: fn(&CanonicalCommand) -> Option<u64>| {
        commands.iter().filter_map(pick).max().unwrap_or(0)
    };
    let mut definition = snapshot
        .definitions()
        .map(|item| item.id().0)
        .max()
        .unwrap_or(0)
        .max(created(|command| match command {
            CanonicalCommand::CreateDefinition { id, .. } => Some(id.0),
            _ => None,
        }));
    let mut feature = snapshot
        .features()
        .map(|item| item.id().0)
        .max()
        .unwrap_or(0)
        .max(created(|command| match command {
            CanonicalCommand::CreateFeature { id, .. } => Some(id.0),
            _ => None,
        }));
    let mut occurrence = snapshot
        .occurrences()
        .map(|item| item.id().0)
        .max()
        .unwrap_or(0)
        .max(created(|command| match command {
            CanonicalCommand::CreateOccurrence { id, .. } => Some(id.0),
            _ => None,
        }));
    let mut next_import = snapshot
        .import_receipts()
        .map(|receipt| receipt.id().0)
        .max()
        .unwrap_or(0)
        .max(created(|command| match command {
            CanonicalCommand::RecordImport(receipt) => Some(receipt.id().0),
            _ => None,
        }));
    // The receipt of each file, by its bytes and how it was read: one
    // already in the document, or one this batch records.
    let mut receipts =
        BTreeMap::<([u8; 32], u64, &'static str), (ImportId, Vec<ImportOutputRef>)>::new();
    let mut new_receipts = Vec::new();
    for part in parts {
        let exhausted =
            || assistant_canonical_rejection(CanonicalError::IdExhausted, "rule_part", &part.name);
        let Some(body) = part.imported().and_then(|import| import.read.as_ref()) else {
            return Err(crate::diagnostics::assistant_planning_rejection(
                "planning.import_unread",
                "rule_part",
                &part.name,
                "The part's STEP file was not read before planning.",
                "Apply the program from its file so Kečup can read the STEP file next to it.",
            ));
        };
        let evidence = &body.evidence;
        let version = parser_version(body);
        let key = (evidence.source_sha256, evidence.source_byte_len, version);
        let import_id = match receipts.get(&key) {
            Some((id, _)) => *id,
            None => {
                let held = snapshot.import_receipts().find(|receipt| {
                    receipt.format() == ImportFormat::Step
                        && receipt.source_sha256() == &evidence.source_sha256
                        && receipt.source_byte_len() == evidence.source_byte_len
                        && receipt.parser_version() == version
                        && receipt.units().source_unit() == evidence.source_unit
                });
                let id = match held {
                    Some(receipt) => receipt.id(),
                    None => {
                        next_import = next_import.checked_add(1).ok_or_else(exhausted)?;
                        new_receipts.push((key, body.clone()));
                        ImportId(next_import)
                    }
                };
                receipts.insert(key, (id, Vec::new()));
                id
            }
        };
        definition = definition.checked_add(1).ok_or_else(exhausted)?;
        feature = feature.checked_add(1).ok_or_else(exhausted)?;
        occurrence = occurrence.checked_add(1).ok_or_else(exhausted)?;
        let (definition_id, feature_id, occurrence_id) = (
            DefinitionId(definition),
            FeatureId(feature),
            OccurrenceId(occurrence),
        );
        if let Some((_, outputs)) = receipts.get_mut(&key) {
            outputs.extend([
                ImportOutputRef::Definition(definition_id),
                ImportOutputRef::Feature(feature_id),
                ImportOutputRef::Occurrence(occurrence_id),
            ]);
        }
        // An assembly part's frame already holds where the assembly places it.
        let transform = Transform::from_matrix(part.transform_matrix())
            .map_err(|error| assistant_canonical_rejection(error, "rule_part", &part.name))?;
        commands.extend([
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: part.name.clone(),
            },
            CanonicalCommand::CreateFeature {
                id: feature_id,
                definition_id,
                name: feature_name(part, body),
                kind: FeatureKind::ImportedExactBody(ImportedExactBodySpec {
                    schema: IMPORTED_EXACT_BODY_SCHEMA_V3.to_owned(),
                    import_id,
                    source_sha256: evidence.source_sha256,
                    source_byte_len: evidence.source_byte_len,
                    source_part_index: body.part_index,
                    result_fingerprint: evidence.result_fingerprint.clone(),
                    body_kind: evidence.body_kind,
                    solid_count: evidence.solid_count,
                    topology_counts: Some(evidence.topology_counts),
                    area_mm2: evidence.area_mm2,
                    volume_mm3: evidence.volume_mm3,
                    bounds_mm: evidence.bounds_mm,
                    backend: evidence.backend.clone(),
                    tolerance: evidence.tolerance.clone(),
                    source_placement: body.placement,
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: part.name.clone(),
                transform,
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]);
    }
    // Each new receipt after the bodies it lists, which must exist by then.
    for (key, body) in new_receipts {
        let Some((import_id, mut outputs)) = receipts.remove(&key) else {
            continue;
        };
        outputs.sort_unstable();
        outputs.truncate(ketchup_model::import::MAX_IMPORT_RECEIPT_OUTPUTS);
        let receipt = ImportReceipt::new(
            import_id,
            ImportFormat::Step,
            body.evidence.source_sha256,
            body.evidence.source_byte_len,
            body.file_name.clone(),
            ImportUnitDecision::new(body.evidence.source_unit, ImportUnitAuthority::FileDeclared),
            STEP_PARSER_ID,
            key.2,
            Vec::new(),
            outputs,
        )
        .map_err(|error| {
            assistant_canonical_rejection(
                CanonicalError::InvalidImportReceipt(error),
                "rule_part",
                &body.file_name,
            )
        })?;
        commands.push(CanonicalCommand::RecordImport(receipt));
    }
    Ok(())
}
