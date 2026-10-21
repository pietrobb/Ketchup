// A count or length written in fewer bits than it holds would be read back as
// another number, so the writers convert with `try_from` (PS-10).
#![deny(clippy::cast_possible_truncation)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DocumentStore, EvaluationIdentity,
    EvaluatorNode, FeatureId, FeatureKind, FeatureParameterFreshnessAudit, NodeId,
    ProposalPrincipal, Revision, RevisionOrigin, Snapshot, ValidatedSnapshot,
};
use crate::graph::SlotResolution;

pub(crate) mod history_chunks;
mod legacy;
mod pipeline;
mod work_recovery;
mod work_recovery_steps;

pub use pipeline::{
    PreparedDocument, Publish, prepare_document, publish_document, publish_work_recovery,
    save_atomic,
};
#[cfg(feature = "testing")]
pub use work_recovery::headless_with_parent_sync_failure;
pub use work_recovery::{WorkRecoveryLock, check_work_recovery_identity, set_aside_work_recovery};
mod payload_error;
pub(crate) mod sheet_metal_v1;
pub(crate) mod snapshot_codec;
mod snapshot_encoding;

pub use legacy::LegacyError;
pub use payload_error::{PayloadError, PayloadLocation, PayloadSection};

const MAGIC: &[u8; 10] = b"KETCHUPDOC";
const CONTAINER_MAGIC: &[u8; 10] = b"KETCHUPCTR";
const CONTAINER_SCHEMA: u16 = 1;
/// Container entry holding the JSON of the program that owns the current revision.
const PROGRAM_ENTRY: &str = "program.json";
const HISTORY_MAGIC: &[u8; 10] = b"KETCHUPHST";
const HISTORY_SCHEMA: u16 = 5;
const WORK_RECOVERY_MAGIC: &[u8; 10] = b"KETCHUPWRK";
/// Format 2: the whole file, then the steps appended per edit (idea #49).
const WORK_RECOVERY_SCHEMA: u16 = 2;
/// Most revisions a native file's history may hold; files written before the shared
/// [`crate::tolerance::limits::UNDO_REVISIONS`] can hold this many.
const FILE_HISTORY_REVISIONS: u32 = 4_096;
/// What the chunks of a history may assemble to. Chunks shared between revisions
/// keep `history.bin` small, so without these a small file could name gigabytes.
/// Writing and opening a work-recovery checkpoint check the same budget, so one
/// that is written also opens; past it, the checkpoint keeps the current revision only.
#[derive(Clone, Copy)]
struct HistoryBudget {
    /// Revision snapshots, the current one aside, each assembled only while it
    /// loads (PS-1).
    snapshots: u64,
    /// Distinct programs, which an opened history holds all at once, each once
    /// however many revisions keep it (PS-2).
    programs: u64,
    /// Counts and string lengths of `history.bin` together, which its reader
    /// counts like every reader (PS-9).
    items: u32,
}
const HISTORY_BUDGET: HistoryBudget = HistoryBudget {
    snapshots: 2 * 1024 * 1024 * 1024,
    programs: 256 * 1024 * 1024,
    items: MAX_COLLECTION_ITEMS,
};
/// The entries the budget is reported for.
const HISTORY_REVISIONS_ENTRY: &str = "the revisions of history.bin";
const HISTORY_PROGRAMS_ENTRY: &str = "the programs of history.bin";
pub const CURRENT_SCHEMA: u16 = snapshot_codec::SNAPSHOT_FORMAT;

const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_COLLECTION_ITEMS: u32 = 500_000;
const MAX_CONTAINER_ENTRIES: u32 = 4_096;
const MAX_CONTAINER_PATH_BYTES: usize = 1_024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionEntry {
    namespace: String,
    path: String,
    required: bool,
    bytes: Vec<u8>,
}

impl ExtensionEntry {
    pub fn new(
        namespace: impl Into<String>,
        path: impl Into<String>,
        required: bool,
        bytes: Vec<u8>,
    ) -> Result<Self, PersistenceError> {
        let namespace = namespace.into();
        let path = path.into();
        validate_namespace(&namespace)?;
        validate_relative_path(&path)?;
        Ok(Self {
            namespace,
            path,
            required,
            bytes,
        })
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContainerData {
    /// Shared, not copied, by every copy: opening a history makes one per
    /// revision (review 2026-10-17 M-5).
    blobs: Arc<BTreeMap<String, Vec<u8>>>,
    imported_source_blobs: Arc<BTreeSet<String>>,
    extensions: BTreeMap<(String, String), ExtensionEntry>,
}

impl ContainerData {
    pub fn insert_blob(&mut self, bytes: Vec<u8>) -> Result<String, PersistenceError> {
        let hash = crate::graph::sha256_hex(&bytes);
        Arc::make_mut(&mut self.blobs)
            .entry(hash.clone())
            .or_insert(bytes);
        Ok(hash)
    }

    pub fn insert_import_blob(&mut self, bytes: Vec<u8>) -> Result<String, PersistenceError> {
        let hash = self.insert_blob(bytes)?;
        Arc::make_mut(&mut self.imported_source_blobs).insert(hash.clone());
        Ok(hash)
    }

    pub fn insert_extension(&mut self, entry: ExtensionEntry) -> Result<(), PersistenceError> {
        let key = (entry.namespace.clone(), entry.path.clone());
        match self.extensions.entry(key) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(entry);
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(_) => {
                Err(PersistenceError::DuplicateContainerEntry)
            }
        }
    }

    pub fn set_extension(&mut self, entry: ExtensionEntry) {
        let key = (entry.namespace.clone(), entry.path.clone());
        self.extensions.insert(key, entry);
    }

    #[must_use]
    pub fn blobs(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.blobs
    }

    pub fn extensions(&self) -> impl Iterator<Item = &ExtensionEntry> {
        self.extensions.values()
    }

    #[must_use]
    pub fn requires_unknown_extension(&self) -> bool {
        self.extensions.values().any(ExtensionEntry::required)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionAudit {
    pub namespace: String,
    pub path: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationLoss {
    pub node_id: NodeId,
    pub field: &'static str,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadDisposition {
    EditableLossless,
    ReviewOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyFeatureKind {
    RoleStringShell,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverrideHealthAudit {
    pub override_id: u64,
    pub stored: SlotResolution,
    pub audited: SlotResolution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadAudit {
    pub source_schema: u16,
    /// The canonical digest as the writing version computed it. A file written before the
    /// current digest definition keeps its old digest here, so records made against the file
    /// (a release manifest) still verify.
    pub source_canonical_digest: String,
    pub migration_losses: Vec<MigrationLoss>,
    pub override_health: Vec<OverrideHealthAudit>,
    pub feature_parameter_freshness: Vec<FeatureParameterFreshnessAudit>,
    pub unknown_extensions: Vec<ExtensionAudit>,
    /// Everything the open did other than read the file as it is, in order; the
    /// window and the session show each one.
    pub notices: Vec<LoadNotice>,
}

/// What an open did in place of reading the file as it is (review 2026-10-15 S1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadNotice {
    /// The file failed; its `.recovery` backup at `path` opened in its place.
    OpenedBackup { path: PathBuf },
    /// The unsaved work of the checkpoint at `path` beside the file opened.
    OpenedWorkRecovery { path: PathBuf },
    /// The saved Undo history was not restored; the document opened with its
    /// current revision only.
    HistoryDiscarded { reason: String },
    /// The work-recovery checkpoint beside the file could not be used. While it
    /// stays there no session of the file can write its own; the caller that
    /// holds the file's WorkRecoveryLock sets it aside with
    /// [`set_aside_work_recovery`].
    WorkRecoveryUnusable { reason: String },
    /// A step of the opened work-recovery checkpoint did not read (a crash cut
    /// it short): the work up to the step before it opened, without the edits
    /// of that step and of any after it.
    WorkRecoveryStepDamaged { reason: String },
}

impl LoadAudit {
    /// Why the saved Undo history was not restored.
    #[must_use]
    pub fn history_discarded(&self) -> Option<&str> {
        self.notices.iter().find_map(|notice| match notice {
            LoadNotice::HistoryDiscarded { reason } => Some(reason.as_str()),
            _ => None,
        })
    }
}

pub struct ReviewCandidate {
    snapshot: Snapshot,
    audit: Box<LoadAudit>,
    container_data: Box<ContainerData>,
    /// The program the file stores beside the document (`program.json`); a
    /// migration keeps it, as opening the file editable would.
    program: Option<Arc<crate::document::RuleProgramSource>>,
}

impl ReviewCandidate {
    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    #[must_use]
    pub const fn audit(&self) -> &LoadAudit {
        &self.audit
    }
    #[must_use]
    pub const fn container_data(&self) -> &ContainerData {
        &self.container_data
    }

    pub fn confirm_semantic_migration(&self) -> Result<ConfirmedMigration, PersistenceError> {
        if self.container_data.requires_unknown_extension() {
            return Err(PersistenceError::MigrationNotConfirmable(
                MigrationBlocker::UnknownExtension,
            ));
        }
        if self.audit.migration_losses.is_empty() {
            return Err(PersistenceError::MigrationNotConfirmable(
                MigrationBlocker::NoSemanticLoss,
            ));
        }
        if self
            .audit
            .override_health
            .iter()
            .any(|entry| entry.audited != SlotResolution::Resolved || entry.audited != entry.stored)
        {
            return Err(PersistenceError::MigrationNotConfirmable(
                MigrationBlocker::UnresolvedOverride,
            ));
        }

        let mut migrated_nodes = BTreeSet::new();
        let mut commands = Vec::new();
        for loss in &self.audit.migration_losses {
            if loss.field != "dimension.source_token" || !migrated_nodes.insert(loss.node_id) {
                return Err(PersistenceError::MigrationNotConfirmable(
                    MigrationBlocker::UnsupportedLoss,
                ));
            }
            let dimension = self
                .snapshot
                .evaluator_node(loss.node_id)
                .and_then(EvaluatorNode::dimension)
                .cloned()
                .ok_or(PersistenceError::MigrationNotConfirmable(
                    MigrationBlocker::NotAParameterDimension,
                ))?;
            commands.push(CanonicalCommand::SetEvaluatorDimension {
                id: loss.node_id,
                dimension,
            });
        }

        let source_revision_id = self.snapshot.revision_id();
        let mut document =
            DocumentStore::from_product(source_revision_id, self.snapshot.product().clone())?;
        let confirmed = document.apply_batch(&CommandBatch::new(commands))?;
        if let Some(program) = &self.program {
            document.bind_rule_program(program.as_ref().clone())?;
        }
        Ok(ConfirmedMigration {
            document,
            container_data: self.container_data.as_ref().clone(),
            source_schema: self.audit.source_schema,
            source_revision_id,
            confirmed_revision_id: confirmed.id(),
            losses: self.audit.migration_losses.clone(),
        })
    }
}

pub struct ConfirmedMigration {
    document: DocumentStore,
    container_data: ContainerData,
    source_schema: u16,
    source_revision_id: u64,
    confirmed_revision_id: u64,
    losses: Vec<MigrationLoss>,
}

impl ConfirmedMigration {
    #[must_use]
    pub const fn source_schema(&self) -> u16 {
        self.source_schema
    }

    #[must_use]
    pub const fn source_revision_id(&self) -> u64 {
        self.source_revision_id
    }

    #[must_use]
    pub const fn confirmed_revision_id(&self) -> u64 {
        self.confirmed_revision_id
    }

    #[must_use]
    pub fn losses(&self) -> &[MigrationLoss] {
        &self.losses
    }

    pub fn into_parts(self) -> (DocumentStore, ContainerData) {
        (self.document, self.container_data)
    }
}

// Boxing the editable audit would change this public persistence result contract.
#[allow(clippy::large_enum_variant)]
pub enum LoadOutcome {
    Editable {
        document: DocumentStore,
        audit: LoadAudit,
        container_data: ContainerData,
    },
    ReviewOnly(ReviewCandidate),
}

impl LoadOutcome {
    #[must_use]
    pub const fn disposition(&self) -> LoadDisposition {
        match self {
            Self::Editable { .. } => LoadDisposition::EditableLossless,
            Self::ReviewOnly(_) => LoadDisposition::ReviewOnly,
        }
    }
    #[must_use]
    pub const fn is_editable(&self) -> bool {
        matches!(self, Self::Editable { .. })
    }
    #[must_use]
    pub const fn audit(&self) -> &LoadAudit {
        match self {
            Self::Editable { audit, .. } => audit,
            Self::ReviewOnly(candidate) => candidate.audit(),
        }
    }
    /// Where the document came from is said before what decoding it found.
    fn notice(&mut self, notice: LoadNotice) {
        match self {
            Self::Editable { audit, .. } => audit.notices.insert(0, notice),
            Self::ReviewOnly(candidate) => candidate.audit.notices.insert(0, notice),
        }
    }
    #[must_use]
    pub const fn source_schema(&self) -> u16 {
        self.audit().source_schema
    }
    #[must_use]
    pub fn migration_losses(&self) -> &[MigrationLoss] {
        &self.audit().migration_losses
    }
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        match self {
            Self::Editable { document, .. } => document.current(),
            Self::ReviewOnly(candidate) => candidate.snapshot.clone(),
        }
    }
    /// Read-only compatibility accessor. It never exposes the backing `DocumentStore`.
    #[must_use]
    pub fn document(&self) -> Snapshot {
        self.snapshot()
    }
    #[must_use]
    pub const fn editable_document(&self) -> Option<&DocumentStore> {
        match self {
            Self::Editable { document, .. } => Some(document),
            Self::ReviewOnly(_) => None,
        }
    }
    #[must_use]
    pub fn editable_document_mut(&mut self) -> Option<&mut DocumentStore> {
        match self {
            Self::Editable { document, .. } => Some(document),
            Self::ReviewOnly(_) => None,
        }
    }
    pub fn into_editable(self) -> Result<DocumentStore, ReviewCandidate> {
        self.into_editable_with_container()
            .map(|(document, _)| document)
    }
    pub fn into_editable_with_container(
        self,
    ) -> Result<(DocumentStore, ContainerData), ReviewCandidate> {
        match self {
            Self::Editable {
                document,
                container_data,
                ..
            } => Ok((document, container_data)),
            Self::ReviewOnly(candidate) => Err(candidate),
        }
    }
    #[must_use]
    pub const fn container_data(&self) -> &ContainerData {
        match self {
            Self::Editable { container_data, .. } => container_data,
            Self::ReviewOnly(candidate) => candidate.container_data(),
        }
    }
    #[must_use]
    pub const fn review_candidate(&self) -> Option<&ReviewCandidate> {
        match self {
            Self::ReviewOnly(candidate) => Some(candidate),
            Self::Editable { .. } => None,
        }
    }
}

#[must_use]
pub fn save(snapshot: &Snapshot) -> Vec<u8> {
    snapshot_codec::encode(snapshot)
}

/// The encoding of `snapshot`, made once per immutable revision: every work-recovery
/// write re-encodes the whole undo history, and a toggle must not cost all of it.
fn encoded_snapshot(snapshot: &Snapshot) -> Arc<crate::document::EncodedSnapshot> {
    let cache = &snapshot.product().exact_graphs.encoded;
    let revision_id = snapshot.revision_id();
    if let Some(encoded) = cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(&revision_id).cloned())
    {
        return encoded;
    }
    let bytes = save(snapshot);
    let encoded = Arc::new(crate::document::EncodedSnapshot {
        sha256: crate::graph::sha256_bytes(&bytes),
        bytes,
        chunks: std::sync::OnceLock::new(),
    });
    if let Ok(mut cache) = cache.lock() {
        cache.insert(revision_id, Arc::clone(&encoded));
        // A store holds at most this many revisions; the oldest ids are those it
        // dropped, so the product keeps no more encodings than revisions sharing it.
        while cache.len() > 2 * crate::tolerance::limits::UNDO_REVISIONS + 1 {
            cache.pop_first();
        }
    }
    encoded
}

/// The blobs `snapshot` reads: the sources of imported bodies and the lines of underlays.
fn imported_source_blob_hashes(snapshot: &Snapshot) -> BTreeSet<String> {
    snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(
                spec.source_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
            ),
            _ => None,
        })
        .chain(
            snapshot
                .underlays()
                .map(crate::underlay::Underlay::lines_blob),
        )
        .collect()
}

fn write_revision_principal(bytes: &mut Vec<u8>, principal: ProposalPrincipal) {
    match principal {
        ProposalPrincipal::ManualClient => push_u8(bytes, 0),
        ProposalPrincipal::Human(id) => {
            push_u8(bytes, 1);
            push_u64(bytes, id);
        }
        ProposalPrincipal::LocalAssistant => push_u8(bytes, 2),
        ProposalPrincipal::Plugin(id) => {
            push_u8(bytes, 3);
            push_u64(bytes, id);
        }
    }
}

fn write_revision_origin(bytes: &mut Vec<u8>, origin: RevisionOrigin) {
    match origin {
        RevisionOrigin::Initial => push_u8(bytes, 0),
        RevisionOrigin::Principal(principal) => {
            push_u8(bytes, 1);
            write_revision_principal(bytes, principal);
        }
        RevisionOrigin::Rollback {
            principal,
            target_revision,
        } => {
            push_u8(bytes, 2);
            write_revision_principal(bytes, principal);
            push_u64(bytes, target_revision);
        }
    }
}

/// The revision's rule program is the one of the revision before it (format 4).
const HISTORY_RULE_PROGRAM_AS_PREVIOUS: u8 = 2;
/// The revision's rule program JSON as chunk numbers, like a snapshot (format 5,
/// PS-2): a program edit stores only the chunks around the change, not the whole
/// program again.
const HISTORY_RULE_PROGRAM_IN_CHUNKS: u8 = 3;

/// A revision as [`encode_history`] writes it: one of a store, or one a
/// work-recovery checkpoint assembles from its steps.
trait HistoryRevision {
    /// Revision id, batch digest, origin and checkpoint.
    fn fields(&self) -> (u64, &str, RevisionOrigin, Option<&str>);
    /// Whether it has a program and it is that of `previous`.
    fn same_program(&self, previous: &Self) -> bool;
    fn program_json(&self) -> Result<Option<std::borrow::Cow<'_, [u8]>>, PersistenceError>;
    fn encoded(&self) -> Arc<crate::document::EncodedSnapshot>;
}

impl HistoryRevision for Revision {
    fn fields(&self) -> (u64, &str, RevisionOrigin, Option<&str>) {
        (
            self.id(),
            self.batch_digest(),
            self.origin(),
            self.checkpoint(),
        )
    }

    fn same_program(&self, previous: &Self) -> bool {
        self.rule_program().is_some() && previous.rule_program() == self.rule_program()
    }

    fn program_json(&self) -> Result<Option<std::borrow::Cow<'_, [u8]>>, PersistenceError> {
        Ok(encoded_rule_program(self)?.map(std::borrow::Cow::Owned))
    }

    fn encoded(&self) -> Arc<crate::document::EncodedSnapshot> {
        encoded_snapshot(self.snapshot())
    }
}

/// Revision id, batch digest, origin and checkpoint.
fn write_revision_header(
    bytes: &mut Vec<u8>,
    revision: &impl HistoryRevision,
) -> Result<(), PersistenceError> {
    let (id, batch_digest, origin, checkpoint) = revision.fields();
    push_u64(bytes, id);
    push_string(bytes, batch_digest)?;
    write_revision_origin(bytes, origin);
    if let Some(checkpoint) = checkpoint {
        push_u8(bytes, 1);
        push_string(bytes, checkpoint)?;
    } else {
        push_u8(bytes, 0);
    }
    Ok(())
}

/// The program JSON of `revision`, within the limit a reader accepts.
fn encoded_rule_program(revision: &Revision) -> Result<Option<Vec<u8>>, PersistenceError> {
    encoded_program(revision.rule_program())
}

/// The JSON of `source`, within the limit a reader accepts.
fn encoded_program(
    source: Option<&crate::document::RuleProgramSource>,
) -> Result<Option<Vec<u8>>, PersistenceError> {
    let Some(source) = source else {
        return Ok(None);
    };
    let encoded = serde_json::to_vec(source)
        .map_err(|error| PersistenceError::payload(PayloadSection::RuleProgram, error))?;
    if encoded.len() > crate::document::MAX_RULE_PROGRAM_BYTES {
        return Err(PersistenceError::ResourceLimit);
    }
    Ok(Some(encoded))
}

/// A revision's metadata as format 3 wrote it, its program stored whole.
#[cfg(test)]
fn write_format_3_revision_metadata(
    bytes: &mut Vec<u8>,
    revision: &Revision,
) -> Result<(), PersistenceError> {
    write_revision_header(bytes, revision)?;
    if let Some(encoded) = encoded_rule_program(revision)? {
        push_u8(bytes, 1);
        push_u32(bytes, u32_count(encoded.len())?);
        bytes.extend_from_slice(&encoded);
    } else {
        push_u8(bytes, 0);
    }
    Ok(())
}

/// The distinct chunks of a history, numbered in the order they are written.
#[derive(Default)]
struct ChunkTable {
    numbers: BTreeMap<[u8; 32], u32>,
    bytes: Vec<u8>,
}

impl ChunkTable {
    fn add_document_chunk(
        &mut self,
        chunk: &history_chunks::Chunk,
    ) -> Result<(), PersistenceError> {
        if self.numbers.contains_key(&chunk.sha256) {
            return Ok(());
        }
        let next = u32_count(self.numbers.len())?;
        let (start, length) = (u32_count(chunk.range.start)?, u32_count(chunk.range.len())?);
        push_u8(&mut self.bytes, HISTORY_CHUNK_IN_DOCUMENT);
        push_u32(&mut self.bytes, start);
        push_u32(&mut self.bytes, length);
        self.numbers.insert(chunk.sha256, next);
        Ok(())
    }

    /// `bytes` into `record` as its length, SHA-256 and chunk numbers; a chunk
    /// not in the table yet is stored in it.
    fn write(
        &mut self,
        record: &mut Vec<u8>,
        bytes: &[u8],
        sha256: &[u8; 32],
        chunks: &[history_chunks::Chunk],
    ) -> Result<(), PersistenceError> {
        push_u64(record, u64_len(bytes.len())?);
        record.extend_from_slice(sha256);
        push_u32(record, u32_count(chunks.len())?);
        for chunk in chunks {
            let number = if let Some(number) = self.numbers.get(&chunk.sha256) {
                *number
            } else {
                let number = u32_count(self.numbers.len())?;
                let bytes = bytes
                    .get(chunk.range.clone())
                    .ok_or(PersistenceError::LengthOverflow)?;
                push_u8(&mut self.bytes, HISTORY_CHUNK_INLINE);
                push_u32(&mut self.bytes, u32_count(bytes.len())?);
                self.bytes.extend_from_slice(bytes);
                self.numbers.insert(chunk.sha256, number);
                number
            };
            push_u32(record, number);
        }
        Ok(())
    }
}

/// `value` as the `u32` a count or length is written as.
fn u32_count(value: usize) -> Result<u32, PersistenceError> {
    u32::try_from(value).map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)
}

/// `value` as the `u64` a byte length is written as.
fn u64_len(value: usize) -> Result<u64, PersistenceError> {
    u64::try_from(value).map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)
}

/// A history revision whose snapshot is the container's `document.bin`.
const HISTORY_SNAPSHOT_IS_DOCUMENT: u8 = 0;
/// A history revision whose snapshot is a list of the history's chunks.
const HISTORY_SNAPSHOT_IN_CHUNKS: u8 = 1;
/// A history chunk stored in the history itself.
const HISTORY_CHUNK_INLINE: u8 = 0;
/// A history chunk that is a byte range of `document.bin`.
const HISTORY_CHUNK_IN_DOCUMENT: u8 = 1;

/// Format 5: the distinct chunks of all revision snapshots and programs once (those
/// the current snapshot has as ranges of `document.bin`), then every revision with
/// its program and snapshot as chunk numbers; the current revision names
/// `document.bin`, a program equal to the one before is named as such.
fn encode_history<R: HistoryRevision>(
    revisions: &[&R],
    cursor: usize,
    next_revision_id: u64,
    document_bin: &crate::document::EncodedSnapshot,
    budget: HistoryBudget,
) -> Result<Vec<u8>, PersistenceError> {
    #[cfg(feature = "testing")]
    HISTORY_ENCODES.set(HISTORY_ENCODES.get() + 1);
    let count = u32::try_from(revisions.len())
        .map_err(|_: std::num::TryFromIntError| PersistenceError::ResourceLimit)?;
    if count == 0 || count > FILE_HISTORY_REVISIONS || cursor >= revisions.len() {
        return Err(PersistenceError::ResourceLimit);
    }
    let mut table = ChunkTable::default();
    for chunk in document_bin
        .chunks
        .get_or_init(|| history_chunks::chunks(&document_bin.bytes))
    {
        table.add_document_chunk(chunk)?;
    }
    let mut records = Vec::new();
    let mut assembled = 0u64;
    let mut programs = BTreeSet::new();
    let mut program_bytes = 0u64;
    // What the reader counts against its item budget: the revision count, the
    // chunk count, every string length and every chunk list.
    let mut items = u64::from(count);
    for (index, revision) in revisions.iter().enumerate() {
        let previous = index.checked_sub(1).map(|previous| revisions[previous]);
        write_revision_header(&mut records, *revision)?;
        let (_, batch_digest, _, checkpoint) = revision.fields();
        items = count_items(items, batch_digest.len(), budget.items)?;
        if let Some(checkpoint) = checkpoint {
            items = count_items(items, checkpoint.len(), budget.items)?;
        }
        if previous.is_some_and(|previous| revision.same_program(previous)) {
            push_u8(&mut records, HISTORY_RULE_PROGRAM_AS_PREVIOUS);
        } else if let Some(encoded) = revision.program_json()? {
            let checksum = crate::graph::sha256_bytes(&encoded);
            if programs.insert(checksum) {
                program_bytes = check_assembled(
                    HISTORY_PROGRAMS_ENTRY,
                    program_bytes,
                    encoded.len(),
                    budget.programs,
                )?;
            }
            push_u8(&mut records, HISTORY_RULE_PROGRAM_IN_CHUNKS);
            let chunks = history_chunks::chunks(&encoded);
            items = count_items(items, chunks.len(), budget.items)?;
            table.write(&mut records, &encoded, &checksum, &chunks)?;
        } else {
            push_u8(&mut records, 0);
        }
        if index == cursor {
            push_u8(&mut records, HISTORY_SNAPSHOT_IS_DOCUMENT);
            continue;
        }
        let snapshot = revision.encoded();
        assembled = check_assembled(
            HISTORY_REVISIONS_ENTRY,
            assembled,
            snapshot.bytes.len(),
            budget.snapshots,
        )?;
        let chunks = snapshot
            .chunks
            .get_or_init(|| history_chunks::chunks(&snapshot.bytes));
        items = count_items(items, chunks.len(), budget.items)?;
        push_u8(&mut records, HISTORY_SNAPSHOT_IN_CHUNKS);
        table.write(&mut records, &snapshot.bytes, &snapshot.sha256, chunks)?;
        count_items(items, table.numbers.len(), budget.items)?;
    }
    count_items(items, table.numbers.len(), budget.items)?;
    let mut bytes = Vec::with_capacity(table.bytes.len() + records.len() + 32);
    bytes.extend_from_slice(HISTORY_MAGIC);
    push_u16(&mut bytes, HISTORY_SCHEMA);
    push_u32(&mut bytes, count);
    push_u32(&mut bytes, u32_count(cursor)?);
    push_u64(&mut bytes, next_revision_id);
    push_u32(&mut bytes, u32_count(table.numbers.len())?);
    bytes.extend_from_slice(&table.bytes);
    bytes.extend_from_slice(&records);
    Ok(bytes)
}

fn encode_revision_history(
    document: &DocumentStore,
    document_bin: &crate::document::EncodedSnapshot,
) -> Result<Vec<u8>, PersistenceError> {
    encode_history(
        &document.revision_history().collect::<Vec<_>>(),
        document.history_cursor(),
        document.next_revision_id(),
        document_bin,
        HISTORY_BUDGET,
    )
}

/// `assembled` plus `bytes` more of `entry`, within `budget`.
fn check_assembled(
    entry: &str,
    assembled: u64,
    bytes: usize,
    budget: u64,
) -> Result<u64, PersistenceError> {
    let total = assembled.saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    if total > budget {
        return Err(PersistenceError::TooLarge {
            entry: entry.to_owned(),
            bytes: total,
            limit: budget,
        });
    }
    Ok(total)
}

/// `items` plus `more`, within the item budget a reader of them keeps.
fn count_items(items: u64, more: usize, budget: u32) -> Result<u64, PersistenceError> {
    let total = items.saturating_add(u64::try_from(more).unwrap_or(u64::MAX));
    if total > u64::from(budget) {
        return Err(PersistenceError::ResourceLimit);
    }
    Ok(total)
}

pub fn save_container(
    snapshot: &Snapshot,
    container_data: &ContainerData,
) -> Result<Vec<u8>, PersistenceError> {
    save_container_entries(
        &encoded_snapshot(snapshot),
        container_data,
        imported_source_blob_hashes(snapshot),
        None,
        None,
    )
}

pub fn save_document_store(
    document: &DocumentStore,
    container_data: &ContainerData,
) -> Result<Vec<u8>, PersistenceError> {
    let snapshot = document.current();
    let document_bin = encoded_snapshot(&snapshot);
    save_container_entries(
        &document_bin,
        container_data,
        history_imported_sources(document),
        Some(encode_revision_history(document, &document_bin)?),
        encoded_program(document.current_rule_program())?,
    )
}

/// The imported sources any revision of `document` names.
fn history_imported_sources(document: &DocumentStore) -> BTreeSet<String> {
    let mut imported_sources = BTreeSet::new();
    for revision in document.revision_history() {
        imported_sources.extend(imported_source_blob_hashes(revision.snapshot()));
    }
    imported_sources
}

pub fn save_document_store_current_snapshot(
    document: &DocumentStore,
    container_data: &ContainerData,
) -> Result<Vec<u8>, PersistenceError> {
    let snapshot = document.current();
    let document_bin = encoded_snapshot(&snapshot);
    save_container_entries(
        &document_bin,
        container_data,
        imported_source_blob_hashes(&snapshot),
        None,
        encoded_program(document.current_rule_program())?,
    )
}

#[cfg(feature = "testing")]
thread_local! {
    static HISTORY_ENCODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SNAPSHOT_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Runs `operation` and counts the document snapshots it decoded on this thread.
#[cfg(feature = "testing")]
pub fn headless_count_snapshot_decodes<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    let before = SNAPSHOT_DECODES.get();
    let result = operation();
    (result, SNAPSHOT_DECODES.get() - before)
}

/// Runs `operation` and counts the Undo histories it encoded on this thread.
#[cfg(feature = "testing")]
pub fn headless_count_history_encodes<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    let before = HISTORY_ENCODES.get();
    let result = operation();
    (result, HISTORY_ENCODES.get() - before)
}

/// The current revision of `document` saved with `history` as its `history.bin`
/// and `program` as its `program.json`, else the document's own program.
#[cfg(feature = "testing")]
pub(crate) fn save_with_history_entry(
    document: &DocumentStore,
    history: Vec<u8>,
    program: Option<Vec<u8>>,
) -> Vec<u8> {
    save_container_entries(
        &encoded_snapshot(&document.current()),
        &ContainerData::default(),
        BTreeSet::new(),
        Some(history),
        program.or_else(|| {
            encoded_program(document.current_rule_program()).expect("a bound program encodes")
        }),
    )
    .expect("a test document fits the container")
}

fn save_container_entries(
    document_bin: &crate::document::EncodedSnapshot,
    container_data: &ContainerData,
    imported_sources: BTreeSet<String>,
    revision_history: Option<Vec<u8>>,
    program: Option<Vec<u8>>,
) -> Result<Vec<u8>, PersistenceError> {
    let mut entries = side_entries(container_data, &imported_sources)?;
    entries.insert(
        "document.bin".to_owned(),
        (true, document_bin.bytes.clone()),
    );
    if let Some(revision_history) = revision_history {
        entries.insert("history.bin".to_owned(), (true, revision_history));
    }
    // The program that owns the current revision belongs to the document, not
    // to its Undo steps: a history that no longer opens must not take it along.
    if let Some(program) = program {
        entries.insert(PROGRAM_ENTRY.to_owned(), (true, program));
    }
    encode_container(entries)
}

/// The entries a file holds beside its document, history and program: the
/// blobs of `container_data` it keeps and its extensions.
fn side_entries(
    container_data: &ContainerData,
    imported_sources: &BTreeSet<String>,
) -> Result<BTreeMap<String, (bool, Vec<u8>)>, PersistenceError> {
    let mut entries = BTreeMap::new();
    if imported_sources
        .iter()
        .any(|hash| !container_data.blobs.contains_key(hash))
    {
        return Err(PersistenceError::InvalidBlobHash);
    }
    for (hash, bytes) in container_data.blobs.iter() {
        if crate::graph::sha256_hex(bytes) != *hash {
            return Err(PersistenceError::InvalidBlobHash);
        }
        if !container_data.imported_source_blobs.contains(hash) || imported_sources.contains(hash) {
            entries.insert(format!("blobs/{hash}"), (false, bytes.clone()));
        }
    }
    for extension in container_data.extensions.values() {
        let path = format!("extensions/{}/{}", extension.namespace, extension.path);
        if entries
            .insert(path, (extension.required, extension.bytes.clone()))
            .is_some()
        {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
    }
    Ok(entries)
}

/// `entries` (path → required, content) as a native container.
fn encode_container(
    entries: BTreeMap<String, (bool, Vec<u8>)>,
) -> Result<Vec<u8>, PersistenceError> {
    if entries.len() > MAX_CONTAINER_ENTRIES as usize {
        return Err(PersistenceError::ResourceLimit);
    }

    let mut encoded = Vec::new();
    encoded.extend_from_slice(CONTAINER_MAGIC);
    push_u16(&mut encoded, CONTAINER_SCHEMA);
    push_u32(&mut encoded, u32_count(entries.len())?);
    for (path, (required, bytes)) in entries {
        validate_container_path(&path)?;
        push_string(&mut encoded, &path)?;
        push_u8(&mut encoded, u8::from(required));
        push_u64(&mut encoded, u64_len(bytes.len())?);
        encoded.extend_from_slice(&crate::graph::sha256_bytes(&bytes));
        encoded.extend_from_slice(&bytes);
    }
    Ok(encoded)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    byte_len: u64,
    sha256: [u8; 32],
}

impl FileIdentity {
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            byte_len: bytes.len() as u64,
            sha256: crate::graph::sha256_bytes(bytes),
        }
    }

    #[must_use]
    pub fn byte_len(self) -> u64 {
        self.byte_len
    }

    #[must_use]
    pub fn sha256(self) -> [u8; 32] {
        self.sha256
    }
}

pub fn remove_regular_file_if_unchanged(
    path: &Path,
    expected: FileIdentity,
    maximum_bytes: u64,
) -> io::Result<bool> {
    remove_regular_file_if_unchanged_after_check(path, expected, maximum_bytes, || {})
}

fn remove_regular_file_if_unchanged_after_check(
    path: &Path,
    expected: FileIdentity,
    maximum_bytes: u64,
    after_check: impl FnOnce(),
) -> io::Result<bool> {
    if bounded_regular_file_identity(path, maximum_bytes)? != Some(expected) {
        return Ok(false);
    }
    after_check();

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".ketchup-cleanup-")
        .tempfile_in(parent)?;
    let claimed_path = temporary.path().to_path_buf();
    temporary.close()?;
    match fs::rename(path, &claimed_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    }

    if bounded_regular_file_identity(&claimed_path, maximum_bytes)? == Some(expected) {
        fs::remove_file(claimed_path)?;
        return Ok(true);
    }

    match fs::hard_link(&claimed_path, path) {
        Ok(()) => {
            fs::remove_file(claimed_path)?;
            Ok(false)
        }
        Err(error) => Err(io::Error::new(
            error.kind(),
            CleanupRaced {
                claimed_path,
                error,
            },
        )),
    }
}

fn bounded_regular_file_identity(
    path: &Path,
    maximum_bytes: u64,
) -> io::Result<Option<FileIdentity>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() || metadata.len() > maximum_bytes {
        return Ok(None);
    }
    let file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > maximum_bytes {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum_bytes {
        return Ok(None);
    }
    Ok(Some(FileIdentity::from_bytes(&bytes)))
}

pub fn read_native_document_file(path: impl AsRef<Path>) -> Result<Vec<u8>, FilePersistenceError> {
    Ok(fs::read(path)?)
}

pub fn read_native_document_identity(
    path: impl AsRef<Path>,
) -> Result<FileIdentity, FilePersistenceError> {
    read_native_document_file(path).map(|bytes| FileIdentity::from_bytes(&bytes))
}

/// Writes the native file `bytes` at `path` under the save lock as `mode`
/// allows, keeping an intact replaced file as its `.recovery`.
fn publish_bytes(
    path: &Path,
    bytes: &[u8],
    mode: Publish,
    after_compare: impl FnOnce(),
    sync_parent: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), FilePersistenceError> {
    verify_checksums(bytes).map_err(FilePersistenceError::Format)?;
    let _lock = lock_saves(path)?;
    let expected = match mode {
        Publish::Replace => None,
        Publish::IfUnchanged(identity) => Some(identity),
        Publish::IfAbsent => {
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(FilePersistenceError::Io(error)),
                Ok(_) => return Err(FilePersistenceError::ExternalConflict),
            }
            let before_persist = || {
                after_compare();
                Ok(())
            };
            return write_atomic_after_prepare(path, bytes, false, before_persist, sync_parent);
        }
    };
    match read_native_document_file(path) {
        Ok(previous)
            if expected.is_some_and(|identity| identity != FileIdentity::from_bytes(&previous)) =>
        {
            return Err(FilePersistenceError::ExternalConflict);
        }
        Ok(previous) if verify_checksums(&previous).is_ok() => {
            write_atomic(&recovery_path(path), &previous)?;
        }
        Err(error @ FilePersistenceError::Format(PersistenceError::ResourceLimit)) => {
            return Err(error);
        }
        Err(_) if expected.is_some() => return Err(FilePersistenceError::ExternalConflict),
        _ => {}
    }
    write_atomic_after_prepare(
        path,
        bytes,
        true,
        || {
            after_compare();
            if expected.is_some_and(|expected| {
                read_native_document_identity(path).map_or(true, |observed| observed != expected)
            }) {
                return Err(FilePersistenceError::ExternalConflict);
            }
            Ok(())
        },
        sync_parent,
    )
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FilePersistenceError> {
    write_atomic_after_prepare(path, bytes, true, || Ok(()), sync_parent_directory)
}

/// Writes `bytes` to a temporary file beside `path` and moves it there, over an
/// existing file only when `replace`; without it an existing `path` is a conflict.
fn write_atomic_after_prepare(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    before_persist: impl FnOnce() -> Result<(), FilePersistenceError>,
    sync_parent: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), FilePersistenceError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    before_persist()?;
    let persisted = if replace {
        temporary.persist(path)
    } else {
        temporary.persist_noclobber(path)
    };
    persisted.map_err(|error| match error.error.kind() {
        io::ErrorKind::AlreadyExists if !replace => FilePersistenceError::ExternalConflict,
        _ => FilePersistenceError::Io(error.error),
    })?;
    sync_parent(parent).map_err(|error| FilePersistenceError::Published {
        path: path.to_owned(),
        identity: FileIdentity::from_bytes(bytes),
        error,
    })?;
    Ok(())
}

fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    #[cfg(feature = "testing")]
    if work_recovery::FAIL_PARENT_SYNC.get() {
        return Err(io::Error::other("injected parent sync failure"));
    }
    #[cfg(unix)]
    {
        fs::File::open(parent)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}

#[must_use]
pub fn work_recovery_path(path: &Path) -> PathBuf {
    let mut recovery = path.as_os_str().to_os_string();
    recovery.push(".work-recovery");
    PathBuf::from(recovery)
}

pub fn clear_work_recovery(
    path: &Path,
    expected: Option<FileIdentity>,
) -> Result<bool, FilePersistenceError> {
    clear_work_recovery_after_compare(path, expected, || {})
}

fn clear_work_recovery_after_compare(
    path: &Path,
    expected: Option<FileIdentity>,
    after_compare: impl FnOnce(),
) -> Result<bool, FilePersistenceError> {
    let _lock = lock_saves(path)?;
    let recovery_path = work_recovery_path(path);
    let metadata = match fs::symlink_metadata(&recovery_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(FilePersistenceError::Io(error)),
    };
    if !metadata.file_type().is_file() {
        return Err(FilePersistenceError::NotARegularFile(recovery_path));
    }
    let Some(expected) = expected else {
        return Ok(false);
    };
    remove_regular_file_if_unchanged_after_check(&recovery_path, expected, u64::MAX, after_compare)
        .map_err(FilePersistenceError::Io)
}

fn recovery_path(path: &Path) -> PathBuf {
    let mut recovery = path.as_os_str().to_os_string();
    recovery.push(".recovery");
    PathBuf::from(recovery)
}

/// Holds the save lock of `path` until the returned file is dropped.
fn lock_saves(path: &Path) -> io::Result<fs::File> {
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(save_lock_path(path))?;
    lock_file.lock()?;
    Ok(lock_file)
}

fn save_lock_path(path: &Path) -> PathBuf {
    let mut lock = path.as_os_str().to_os_string();
    lock.push(".save-lock");
    PathBuf::from(lock)
}

/// What the open found at the work-recovery path of a file it read.
enum WorkRecoveryRead {
    Absent,
    Loaded(Box<LoadedFile>),
    Unusable(UnusableWorkRecovery),
}

/// Why a work-recovery checkpoint beside a file was not opened.
#[derive(Debug)]
enum UnusableWorkRecovery {
    /// Not a checkpoint format this version reads.
    UnknownFormat,
    /// Made from another version of the file than the one on disk.
    OtherVersion,
    /// Its framing is cut short or inconsistent.
    Damaged,
    /// The document in it does not open.
    Document(PersistenceError),
    /// It could not be read.
    Unreadable(io::Error),
}

impl fmt::Display for UnusableWorkRecovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat => {
                formatter.write_str("it is not a work-recovery checkpoint this version reads")
            }
            Self::OtherVersion => {
                formatter.write_str("it was made from another version of the file")
            }
            Self::Damaged => formatter.write_str("the checkpoint is damaged"),
            Self::Document(error) => write!(formatter, "its document does not open: {error}"),
            Self::Unreadable(error) => write!(formatter, "it cannot be read: {error}"),
        }
    }
}

fn try_load_work_recovery(
    path: &Path,
    base_identity: FileIdentity,
) -> io::Result<WorkRecoveryRead> {
    use WorkRecoveryRead::Unusable;
    let source_path = work_recovery_path(path);
    let mut file = match fs::File::open(&source_path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(WorkRecoveryRead::Absent);
        }
        Err(error) => return Ok(Unusable(UnusableWorkRecovery::Unreadable(error))),
    };
    let damaged = || Unusable(UnusableWorkRecovery::Damaged);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut reader = Reader::new(&bytes);
    if reader.take(WORK_RECOVERY_MAGIC.len()).ok() != Some(WORK_RECOVERY_MAGIC) {
        return Ok(Unusable(UnusableWorkRecovery::UnknownFormat));
    }
    let Some(schema) = reader
        .u16()
        .ok()
        .filter(|schema| (1..=WORK_RECOVERY_SCHEMA).contains(schema))
    else {
        return Ok(Unusable(UnusableWorkRecovery::UnknownFormat));
    };
    let Some(byte_len) = reader.u64().ok() else {
        return Ok(damaged());
    };
    let Some(sha256) = reader.take(32).ok().and_then(|value| value.try_into().ok()) else {
        return Ok(damaged());
    };
    if (FileIdentity { byte_len, sha256 }) != base_identity {
        return Ok(Unusable(UnusableWorkRecovery::OtherVersion));
    }
    let Some(payload_len) = reader
        .u64()
        .ok()
        .and_then(|value| usize::try_from(value).ok())
    else {
        return Ok(damaged());
    };
    let Some(payload) = reader.take(payload_len).ok() else {
        return Ok(damaged());
    };
    let steps = &bytes[reader.cursor..];
    if schema == 1 && !steps.is_empty() {
        return Ok(damaged());
    }
    let (opened, damaged_step) = if steps.is_empty() {
        (payload.to_vec(), None)
    } else {
        match work_recovery_steps::assemble_checkpoint(payload, steps) {
            Ok(assembled) => assembled,
            Err(error) => return Ok(Unusable(UnusableWorkRecovery::Document(error))),
        }
    };
    let mut outcome = match load_with_history(&opened, true) {
        Ok(outcome) => outcome,
        Err(error) => return Ok(Unusable(UnusableWorkRecovery::Document(error))),
    };
    // Each notice goes before those already there.
    if let Some(reason) = damaged_step {
        outcome.notice(LoadNotice::WorkRecoveryStepDamaged { reason });
    }
    outcome.notice(LoadNotice::OpenedWorkRecovery {
        path: source_path.clone(),
    });
    Ok(WorkRecoveryRead::Loaded(Box::new(LoadedFile {
        outcome,
        source_path,
        source_bytes: opened,
        work_recovery_identity: Some(FileIdentity::from_bytes(&bytes)),
        work_recovery_base: Some(base_identity),
    })))
}

pub struct LoadedFile {
    outcome: LoadOutcome,
    source_path: PathBuf,
    source_bytes: Vec<u8>,
    work_recovery_identity: Option<FileIdentity>,
    work_recovery_base: Option<FileIdentity>,
}

impl LoadedFile {
    #[must_use]
    pub fn outcome(&self) -> &LoadOutcome {
        &self.outcome
    }

    #[must_use]
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    #[must_use]
    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    #[must_use]
    pub fn work_recovery_identity(&self) -> Option<FileIdentity> {
        self.work_recovery_identity
    }

    /// When the open came from a work-recovery checkpoint, the identity of
    /// the file it was made from: later checkpoints of the work are made from
    /// it too.
    #[must_use]
    pub fn work_recovery_base(&self) -> Option<FileIdentity> {
        self.work_recovery_base
    }

    #[must_use]
    pub fn into_parts(self) -> (LoadOutcome, PathBuf, Vec<u8>, Option<FileIdentity>) {
        (
            self.outcome,
            self.source_path,
            self.source_bytes,
            self.work_recovery_identity,
        )
    }
}

pub fn load_file(path: impl AsRef<Path>) -> Result<LoadOutcome, FilePersistenceError> {
    Ok(load_file_with_source(path)?.outcome)
}

pub fn load_file_with_source(path: impl AsRef<Path>) -> Result<LoadedFile, FilePersistenceError> {
    let path = path.as_ref();
    match read_native_document_file(path) {
        Ok(bytes) => match load(&bytes) {
            Ok(mut outcome) => {
                let identity = FileIdentity::from_bytes(&bytes);
                let unusable = match try_load_work_recovery(path, identity) {
                    Ok(WorkRecoveryRead::Loaded(recovered)) => return Ok(*recovered),
                    Ok(WorkRecoveryRead::Absent) => None,
                    Ok(WorkRecoveryRead::Unusable(reason)) => Some(reason),
                    Err(error) => Some(UnusableWorkRecovery::Unreadable(error)),
                };
                if let Some(reason) = unusable {
                    outcome.notice(LoadNotice::WorkRecoveryUnusable {
                        reason: reason.to_string(),
                    });
                }
                Ok(LoadedFile {
                    outcome,
                    source_path: path.to_owned(),
                    source_bytes: bytes,
                    work_recovery_identity: None,
                    work_recovery_base: None,
                })
            }
            // A file that needs migration or that a newer Kečup wrote is not
            // damaged; its older recovery would overwrite the newer work on Save.
            Err(
                primary_error @ (PersistenceError::LegacyFeatureRequiresMigration { .. }
                | PersistenceError::UnsupportedSchema(_)
                | PersistenceError::UnsupportedContainerSchema(_)
                | PersistenceError::UnsupportedContainerEntry(_)
                | PersistenceError::UnsupportedHistorySchema(_)
                | PersistenceError::UnknownFields(_)),
            ) => Err(FilePersistenceError::Format(primary_error)),
            Err(primary_error) => open_backup(path, FilePersistenceError::Format(primary_error)),
        },
        Err(error @ FilePersistenceError::Format(PersistenceError::ResourceLimit)) => Err(error),
        Err(primary_error @ FilePersistenceError::Io(_)) => open_backup(path, primary_error),
        Err(error) => Err(error),
    }
}

/// Opens the `.recovery` backup of `path` in place of the file, which failed with
/// `primary`. Without a backup the answer is `primary`; when the backup fails too, the
/// answer names both failures.
fn open_backup(
    path: &Path,
    primary: FilePersistenceError,
) -> Result<LoadedFile, FilePersistenceError> {
    let backup_path = recovery_path(path);
    let backup = match read_native_document_file(&backup_path) {
        Ok(bytes) => match load(&bytes) {
            Ok(mut outcome) => {
                outcome.notice(LoadNotice::OpenedBackup {
                    path: backup_path.clone(),
                });
                return Ok(LoadedFile {
                    outcome,
                    source_path: backup_path,
                    source_bytes: bytes,
                    work_recovery_identity: None,
                    work_recovery_base: None,
                });
            }
            Err(error) => FilePersistenceError::Format(error),
        },
        Err(FilePersistenceError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(primary);
        }
        Err(error) => error,
    };
    Err(FilePersistenceError::BackupUnusable {
        primary: Box::new(primary),
        backup_path,
        backup: Box::new(backup),
    })
}

/// The document `bytes` hold at their current revision; an Undo history they
/// hold is ignored.
pub fn load(bytes: &[u8]) -> Result<LoadOutcome, PersistenceError> {
    load_with_history(bytes, false)
}

/// [`load`] that also restores the Undo history a work-recovery checkpoint holds.
pub(crate) fn load_with_history(
    bytes: &[u8],
    restore_history: bool,
) -> Result<LoadOutcome, PersistenceError> {
    if bytes.starts_with(CONTAINER_MAGIC) {
        return load_container(bytes, restore_history);
    }
    load_document(bytes, ContainerData::default(), &mut BTreeMap::new())
}

/// Whether `bytes` are an intact native container: its framing, the checksum of
/// every entry and a required `document.bin`. Save checks what it writes and
/// what it keeps as `.recovery` this way, without decoding either document
/// (review PS-6, 2026-10-15 S2). A file from before the container is decoded.
fn verify_checksums(bytes: &[u8]) -> Result<(), PersistenceError> {
    if !bytes.starts_with(CONTAINER_MAGIC) {
        return load(bytes).map(drop);
    }
    match container_entries(bytes)?.get("document.bin") {
        Some((true, _)) => Ok(()),
        Some((false, _)) => Err(PersistenceError::DocumentEntryNotRequired),
        None => Err(PersistenceError::MissingDocumentEntry),
    }
}

/// The entries of a native container whose framing and checksums hold.
fn container_entries(bytes: &[u8]) -> Result<BTreeMap<String, (bool, Vec<u8>)>, PersistenceError> {
    let mut entries = BTreeMap::<String, (bool, Vec<u8>)>::new();
    for entry in container_directory(bytes)? {
        if crate::graph::sha256_bytes(entry.content) != entry.checksum {
            return Err(PersistenceError::ContainerChecksumMismatch(entry.path));
        }
        if entries
            .insert(entry.path, (entry.required, entry.content.to_vec()))
            .is_some()
        {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
    }
    Ok(entries)
}

/// One entry of a native container as its framing names it.
struct ContainerEntry<'a> {
    path: String,
    required: bool,
    checksum: [u8; 32],
    content: &'a [u8],
}

/// The entries of a native container whose framing holds, their checksums not
/// verified: for bytes whose file identity was already checked.
fn container_directory(bytes: &[u8]) -> Result<Vec<ContainerEntry<'_>>, PersistenceError> {
    let mut reader = Reader::new(bytes);
    if reader.take(CONTAINER_MAGIC.len())? != CONTAINER_MAGIC {
        return Err(PersistenceError::InvalidContainerMagic);
    }
    let schema = reader.u16()?;
    if schema != CONTAINER_SCHEMA {
        return Err(PersistenceError::UnsupportedContainerSchema(schema));
    }
    let entry_count = reader.count_with_limit(MAX_CONTAINER_ENTRIES)?;
    let mut entries = Vec::new();
    for _ in 0..entry_count {
        let path = reader.string()?;
        validate_container_path(&path)?;
        let required = reader.boolean()?;
        let length = usize::try_from(reader.u64()?)
            .map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)?;
        // Every stored length fits in the rest of the file, its checksum first.
        if length > (bytes.len() - reader.cursor).saturating_sub(32) {
            return Err(PersistenceError::Truncated);
        }
        let checksum: [u8; 32] = reader
            .take(32)?
            .try_into()
            .map_err(|_: std::array::TryFromSliceError| PersistenceError::Truncated)?;
        entries.push(ContainerEntry {
            path,
            required,
            checksum,
            content: reader.take(length)?,
        });
    }
    if !reader.is_finished() {
        return Err(PersistenceError::TrailingBytes);
    }
    Ok(entries)
}

fn load_container(bytes: &[u8], restore_history: bool) -> Result<LoadOutcome, PersistenceError> {
    let mut entries = container_entries(bytes)?;
    let (document_required, document) = entries
        .remove("document.bin")
        .ok_or(PersistenceError::MissingDocumentEntry)?;
    if !document_required {
        return Err(PersistenceError::DocumentEntryNotRequired);
    }
    let revision_history = match entries.remove("history.bin") {
        Some((true, content)) => Some(content),
        Some((false, _)) => return Err(PersistenceError::HistoryEntryNotRequired),
        None => None,
    };
    let program = match entries.remove(PROGRAM_ENTRY) {
        Some((true, content)) => Some(Arc::new(decode_rule_program(&content)?)),
        Some((false, _)) => return Err(PersistenceError::ProgramEntryNotRequired),
        None => None,
    };
    let mut container_data = ContainerData::default();
    for (path, (required, content)) in entries {
        if let Some(hash) = path.strip_prefix("blobs/") {
            if required || hash.len() != 64 || crate::graph::sha256_hex(&content) != hash {
                return Err(PersistenceError::InvalidBlobHash);
            }
            if Arc::make_mut(&mut container_data.blobs)
                .insert(hash.to_owned(), content)
                .is_some()
            {
                return Err(PersistenceError::DuplicateContainerEntry);
            }
        } else if let Some(extension_path) = path.strip_prefix("extensions/") {
            let (namespace, relative_path) = extension_path
                .split_once('/')
                .ok_or_else(|| PersistenceError::InvalidContainerPath(path.clone()))?;
            container_data.insert_extension(ExtensionEntry::new(
                namespace,
                relative_path,
                required,
                content,
            )?)?;
        } else {
            return Err(PersistenceError::UnsupportedContainerEntry(path));
        }
    }
    let history_container_data = ContainerData {
        blobs: Arc::clone(&container_data.blobs),
        imported_source_blobs: Arc::new(BTreeSet::new()),
        extensions: BTreeMap::new(),
    };
    let mut outcome = load_document(&document, container_data, &mut BTreeMap::new())?;
    match (&program, &mut outcome) {
        (Some(program), LoadOutcome::Editable { document, .. }) => {
            document.restore_opened_rule_program(Arc::clone(program));
        }
        (Some(program), LoadOutcome::ReviewOnly(candidate)) => {
            candidate.program = Some(Arc::clone(program));
        }
        (None, _) => {}
    }
    if let Some(revision_history) = revision_history {
        // A file opens without Undo steps; only a work-recovery checkpoint restores
        // them. The current revision was verified on its own; a history that no
        // longer decodes (a damaged entry) costs the Undo steps, not the document.
        let restored = restore_history.then(|| {
            decode_revision_history(
                &revision_history,
                &document,
                &outcome,
                &history_container_data,
                HISTORY_BUDGET,
            )
        });
        if program.is_none() && !matches!(restored, Some(Ok(_))) {
            // Without program.json (files before 2026-10-15) the history is the
            // program's only home: the revision the file opens at keeps its program
            // (PS-1); a current program that does not decode is refused by name.
            match current_history_program(
                &revision_history,
                &document,
                outcome.snapshot().revision_id(),
            ) {
                Ok(Ok(Some(current))) => match &mut outcome {
                    LoadOutcome::Editable { document, .. } => {
                        document.restore_opened_rule_program(Arc::new(current));
                    }
                    LoadOutcome::ReviewOnly(candidate) => {
                        candidate.program = Some(Arc::new(current));
                    }
                },
                Ok(Ok(None)) => {}
                Ok(Err(current)) => return Err(current),
                // The current program cannot be read apart from the steps, so
                // one that breaks its rules may be the current one.
                Err(error) => match &restored {
                    Some(Err(PersistenceError::InvalidRuleProgram(invalid))) => {
                        return Err(PersistenceError::InvalidRuleProgram(invalid.clone()));
                    }
                    Some(_) => {}
                    None => outcome.notice(LoadNotice::HistoryDiscarded {
                        reason: error.to_string(),
                    }),
                },
            }
        }
        let Some(restored) = restored else {
            return Ok(outcome);
        };
        let discarded = |reason: String| LoadNotice::HistoryDiscarded { reason };
        match (&mut outcome, restored) {
            // A history a newer Kečup wrote is not damaged: opened without it,
            // the next Save would overwrite its steps with this one revision.
            (_, Err(error @ PersistenceError::UnsupportedHistorySchema(schema)))
                if schema > HISTORY_SCHEMA =>
            {
                return Err(error);
            }
            // The history must end at the program the file stores beside the
            // document; one that ends elsewhere is not this document's history.
            (LoadOutcome::Editable { audit, .. }, Ok(restored))
                if program.is_some() && restored.current_rule_program() != program.as_deref() =>
            {
                audit.notices.push(discarded(
                    PersistenceError::InvalidRevisionHistory.to_string(),
                ));
            }
            (LoadOutcome::Editable { document, .. }, Ok(restored)) => *document = restored,
            (LoadOutcome::Editable { audit, .. }, Err(error)) => {
                audit.notices.push(discarded(error.to_string()));
            }
            // A review never restores Undo steps, so only the reason is kept.
            (LoadOutcome::ReviewOnly(candidate), restored) => {
                if let Err(error) = restored {
                    candidate.audit.notices.push(discarded(error.to_string()));
                }
            }
        }
    }
    Ok(outcome)
}

fn read_revision_principal(reader: &mut Reader<'_>) -> Result<ProposalPrincipal, PersistenceError> {
    match reader.u8()? {
        0 => Ok(ProposalPrincipal::ManualClient),
        1 => Ok(ProposalPrincipal::Human(reader.u64()?)),
        2 => Ok(ProposalPrincipal::LocalAssistant),
        3 => Ok(ProposalPrincipal::Plugin(reader.u64()?)),
        _ => Err(PersistenceError::InvalidRevisionHistory),
    }
}

fn read_revision_origin(reader: &mut Reader<'_>) -> Result<RevisionOrigin, PersistenceError> {
    match reader.u8()? {
        0 => Ok(RevisionOrigin::Initial),
        1 => Ok(RevisionOrigin::Principal(read_revision_principal(reader)?)),
        2 => Ok(RevisionOrigin::Rollback {
            principal: read_revision_principal(reader)?,
            target_revision: reader.u64()?,
        }),
        _ => Err(PersistenceError::InvalidRevisionHistory),
    }
}

fn decode_revision_history(
    bytes: &[u8],
    document_bytes: &[u8],
    expected_current: &LoadOutcome,
    container_data: &ContainerData,
    budget: HistoryBudget,
) -> Result<DocumentStore, PersistenceError> {
    #[cfg(test)]
    tests::HISTORY_DECODES.with(|count| count.set(count.get() + 1));
    let ParsedHistory {
        schema,
        cursor,
        next_revision_id,
        chunks,
        records,
    } = read_history_records(bytes, document_bytes, budget, false)?;
    // A program the history names again is decoded once and shared.
    let mut decoded = BTreeMap::new();
    let rule_programs = records
        .iter()
        .map(|record| {
            record
                .rule_program
                .map(|program| {
                    if let Some(shared) = decoded.get(&program) {
                        return Ok(Arc::clone(shared));
                    }
                    let source = Arc::new(decode_history_program(program, &chunks)?);
                    decoded.insert(program, Arc::clone(&source));
                    Ok(source)
                })
                .transpose()
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    decode_history_revisions(
        ParsedHistory {
            schema,
            cursor,
            next_revision_id,
            chunks,
            records,
        },
        rule_programs,
        expected_current,
        container_data,
    )
}

/// The program of the revision a history names as current, read without the
/// other revisions' snapshots or programs. A file written before
/// `program.json` keeps its program only there, so a history that does not
/// open whole must not take that program with it (review 2026-10-15 status PS-1).
/// The outer error is a history that cannot be read as far as that revision,
/// the inner one a program that is there but does not decode.
fn current_history_program(
    bytes: &[u8],
    document_bytes: &[u8],
    expected_revision_id: u64,
) -> Result<Result<Option<crate::document::RuleProgramSource>, PersistenceError>, PersistenceError>
{
    let ParsedHistory {
        cursor,
        chunks,
        records,
        ..
    } = read_history_records(bytes, document_bytes, HISTORY_BUDGET, true)?;
    let current = &records[cursor];
    if current.revision_id != expected_revision_id {
        return Err(PersistenceError::InvalidRevisionHistory);
    }
    Ok(current
        .rule_program
        .map(|program| decode_history_program(program, &chunks))
        .transpose())
}

fn decode_history_program(
    program: HistoryProgram<'_>,
    chunks: &[&[u8]],
) -> Result<crate::document::RuleProgramSource, PersistenceError> {
    decode_rule_program(&program.assemble(chunks)?)
}

/// A history as the file names it: every revision's place, nothing assembled.
struct ParsedHistory<'a> {
    schema: u16,
    cursor: usize,
    next_revision_id: u64,
    chunks: Vec<&'a [u8]>,
    records: Vec<HistoryRecord<'a>>,
}

fn read_history_records<'a>(
    bytes: &'a [u8],
    document_bytes: &'a [u8],
    budget: HistoryBudget,
    current_only: bool,
) -> Result<ParsedHistory<'a>, PersistenceError> {
    let mut reader = Reader::with_item_budget(bytes, budget.items);
    if reader.take(HISTORY_MAGIC.len())? != HISTORY_MAGIC {
        return Err(PersistenceError::InvalidHistoryMagic);
    }
    let schema = reader.u16()?;
    if !(1..=HISTORY_SCHEMA).contains(&schema) {
        return Err(PersistenceError::UnsupportedHistorySchema(schema));
    }
    let count = reader.count_with_limit(FILE_HISTORY_REVISIONS)? as usize;
    let cursor = reader.u32()? as usize;
    let next_revision_id = reader.u64()?;
    if count == 0 || cursor >= count {
        return Err(PersistenceError::InvalidRevisionHistory);
    }
    // Opening a file needs only the program of the revision it opens at; the
    // others are read only to reach the next one and count against no budget.
    let kept = if current_only {
        cursor..=cursor
    } else {
        0..=count - 1
    };
    let mut chunks = Vec::new();
    if schema >= 4 {
        let chunk_count = reader.count_with_limit(MAX_COLLECTION_ITEMS)?;
        chunks.reserve(chunk_count as usize);
        for _ in 0..chunk_count {
            let chunk = match reader.u8()? {
                HISTORY_CHUNK_INLINE => {
                    let length = reader.u32()? as usize;
                    reader.take(length)?
                }
                HISTORY_CHUNK_IN_DOCUMENT => {
                    let start = reader.u32()? as usize;
                    let length = reader.u32()? as usize;
                    document_bytes
                        .get(start..start + length)
                        .ok_or(PersistenceError::InvalidRevisionHistory)?
                }
                _ => return Err(PersistenceError::InvalidRevisionHistory),
            };
            chunks.push(chunk);
        }
    }

    let mut records = Vec::with_capacity(kept.end() - kept.start() + 1);
    let mut assembled = 0u64;
    let mut programs = BTreeSet::new();
    let mut program_bytes = 0u64;
    let mut previous_revision_id = None;
    let mut previous_program = None;
    let mut checkpoint_names = BTreeSet::new();
    for index in 0..count {
        let keep = kept.contains(&index);
        let revision_id = reader.u64()?;
        if previous_revision_id.is_some_and(|previous| revision_id <= previous) {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        let batch_digest = reader.string()?;
        if !batch_digest.is_empty()
            && (batch_digest.len() != 16
                || !batch_digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        let (origin, checkpoint) = if schema == 1 {
            (
                if index == 0 {
                    RevisionOrigin::Initial
                } else {
                    RevisionOrigin::Principal(ProposalPrincipal::ManualClient)
                },
                None,
            )
        } else {
            let origin = read_revision_origin(&mut reader)?;
            let checkpoint = match reader.u8()? {
                0 => None,
                1 => Some(reader.string()?),
                _ => return Err(PersistenceError::InvalidRevisionHistory),
            };
            (origin, checkpoint)
        };
        let rule_program = if schema >= 3 {
            match reader.u8()? {
                0 => None,
                1 => {
                    let length = reader.u32()? as usize;
                    if length > crate::document::MAX_RULE_PROGRAM_BYTES {
                        return Err(PersistenceError::ResourceLimit);
                    }
                    Some(HistoryProgram::Inline(reader.take(length)?))
                }
                HISTORY_RULE_PROGRAM_IN_CHUNKS if schema >= 5 => {
                    let (length, checksum, numbers) =
                        read_chunk_list(&mut reader, crate::document::MAX_RULE_PROGRAM_BYTES)?;
                    Some(HistoryProgram::Chunks {
                        length,
                        checksum,
                        numbers,
                    })
                }
                HISTORY_RULE_PROGRAM_AS_PREVIOUS if schema >= 4 => {
                    Some(previous_program.ok_or(PersistenceError::InvalidRevisionHistory)?)
                }
                _ => return Err(PersistenceError::InvalidRevisionHistory),
            }
        } else {
            None
        };
        previous_program = rule_program;
        // A program the history named before is the same one, counted once.
        if keep
            && let Some(HistoryProgram::Chunks {
                length,
                checksum,
                numbers,
            }) = rule_program
            && programs.insert((length, checksum, numbers))
        {
            program_bytes = check_assembled(
                HISTORY_PROGRAMS_ENTRY,
                program_bytes,
                length,
                budget.programs,
            )?;
        }
        if (index > 0 && matches!(origin, RevisionOrigin::Initial))
            || matches!(origin, RevisionOrigin::Rollback { target_revision, .. } if target_revision >= revision_id)
            || matches!(
                origin,
                RevisionOrigin::Principal(
                    ProposalPrincipal::Human(0) | ProposalPrincipal::Plugin(0)
                )
            )
        {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        if let Some(name) = checkpoint.as_ref()
            && (name.is_empty()
                || name.len() > 80
                || name.trim() != name
                || name.chars().any(char::is_control)
                || !checkpoint_names.insert(name.clone()))
        {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        let encoded_snapshot = read_history_snapshot(
            &mut reader,
            schema,
            (index == cursor).then_some(document_bytes),
        )?;
        previous_revision_id = Some(revision_id);
        if !keep {
            continue;
        }
        // Checked before anything is assembled.
        assembled = check_assembled(
            HISTORY_REVISIONS_ENTRY,
            assembled,
            encoded_snapshot.assembled_len(),
            budget.snapshots,
        )?;
        records.push(HistoryRecord {
            revision_id,
            batch_digest,
            origin,
            checkpoint,
            rule_program,
            encoded_snapshot,
        });
    }
    if !reader.is_finished()
        || previous_revision_id.is_none_or(|revision_id| next_revision_id <= revision_id)
    {
        return Err(PersistenceError::InvalidRevisionHistory);
    }
    Ok(ParsedHistory {
        schema,
        cursor: cursor - kept.start(),
        next_revision_id,
        chunks,
        records,
    })
}

/// The kept revisions of `history` loaded into a store, each with its program.
fn decode_history_revisions(
    history: ParsedHistory<'_>,
    rule_programs: Vec<Option<Arc<crate::document::RuleProgramSource>>>,
    expected_current: &LoadOutcome,
    container_data: &ContainerData,
) -> Result<DocumentStore, PersistenceError> {
    let ParsedHistory {
        schema,
        cursor,
        next_revision_id,
        chunks,
        records,
    } = history;
    let snapshots = if schema >= 4 {
        // Every revision a format-4 history holds was written in the current format,
        // so each loads on its own: on all cores, as a revision of 1800 parts takes ~0.4 s.
        // The current revision is document.bin, which is already loaded and validated.
        let expected_validated = match expected_current {
            LoadOutcome::Editable { document, .. } => document.validated_current(),
            // A review restores no history, but a damaged one is still reported.
            LoadOutcome::ReviewOnly(candidate) => DocumentStore::from_product(
                candidate.snapshot.revision_id(),
                candidate.snapshot.product().clone(),
            )
            .map_err(PersistenceError::InvalidCanonicalData)?
            .validated_current(),
        };
        let expected_digest = expected_current.audit().source_canonical_digest.clone();
        load_in_parallel(&records, |index, record| {
            if index == cursor {
                return Ok((expected_validated.clone(), expected_digest.clone()));
            }
            // Assembled only while it loads, so the history is never all
            // assembled at once.
            let loaded = load_history_snapshot(
                &record.encoded_snapshot.assemble(&chunks)?,
                container_data,
                &mut BTreeMap::new(),
            )?;
            if loaded.0.snapshot().revision_id() != record.revision_id {
                return Err(PersistenceError::InvalidRevisionHistory);
            }
            Ok(loaded)
        })?
    } else {
        // Older histories can hold legacy formats whose migrated digests chain.
        let mut migrated_digests = BTreeMap::new();
        records
            .iter()
            .map(|record| {
                load_history_snapshot(
                    &record.encoded_snapshot.assemble(&chunks)?,
                    container_data,
                    &mut migrated_digests,
                )
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let current_source_digest = Some(snapshots[cursor].1.clone());
    let mut revisions = Vec::with_capacity(records.len());
    let mut document_id = None;
    for ((record, (validated, _)), rule_program) in
        records.into_iter().zip(snapshots).zip(rule_programs)
    {
        let snapshot = validated.snapshot();
        if snapshot.revision_id() != record.revision_id
            || document_id.is_some_and(|id| id != snapshot.document_id())
        {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        document_id = Some(snapshot.document_id());
        revisions.push((
            validated,
            record.batch_digest,
            record.origin,
            record.checkpoint,
            rule_program,
        ));
    }
    // The revision a history names as current is the saved document. Both were written by
    // one version, so they agree on the digest that version computed.
    let current = revisions[cursor].0.snapshot();
    let expected_snapshot = expected_current.snapshot();
    if current.document_id() != expected_snapshot.document_id()
        || current.revision_id() != expected_snapshot.revision_id()
        || current_source_digest.as_deref()
            != Some(expected_current.audit().source_canonical_digest.as_str())
        || (schema < 4
            && expected_current.source_schema() == CURRENT_SCHEMA
            && save(current) != save(&expected_snapshot))
    {
        return Err(PersistenceError::InvalidRevisionHistory);
    }
    Ok(DocumentStore::from_revision_history(
        revisions,
        cursor,
        next_revision_id,
    ))
}

struct HistoryRecord<'a> {
    revision_id: u64,
    batch_digest: String,
    origin: RevisionOrigin,
    checkpoint: Option<String>,
    rule_program: Option<HistoryProgram<'a>>,
    encoded_snapshot: HistorySnapshot<'a>,
}

/// The program of one history revision as the file names it, decoded only if
/// the revision is kept.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HistoryProgram<'a> {
    /// Stored whole, before format 5.
    Inline(&'a [u8]),
    /// Chunk numbers of `length` bytes, from format 5.
    Chunks {
        length: usize,
        checksum: [u8; 32],
        numbers: &'a [u8],
    },
}

impl<'a> HistoryProgram<'a> {
    /// The program JSON, checked against its checksum.
    fn assemble(self, chunks: &[&[u8]]) -> Result<std::borrow::Cow<'a, [u8]>, PersistenceError> {
        match self {
            Self::Inline(bytes) => Ok(std::borrow::Cow::Borrowed(bytes)),
            Self::Chunks {
                length,
                checksum,
                numbers,
            } => HistorySnapshot::Chunks {
                length,
                checksum,
                numbers,
            }
            .assemble(chunks),
        }
    }
}

/// The encoded snapshot of one history revision as the file names it.
enum HistorySnapshot<'a> {
    /// The current revision: `document.bin`, loaded and verified on its own.
    Document(&'a [u8]),
    /// Stored whole, before format 4.
    Whole { bytes: &'a [u8], checksum: [u8; 32] },
    /// Chunk numbers (`u32` each) of `length` bytes, from format 4.
    Chunks {
        length: usize,
        checksum: [u8; 32],
        numbers: &'a [u8],
    },
}

impl<'a> HistorySnapshot<'a> {
    /// The checksum of its bytes; `document` is that of `document.bin`.
    const fn checksum(&self, document: [u8; 32]) -> [u8; 32] {
        match self {
            Self::Document(_) => document,
            Self::Whole { checksum, .. } | Self::Chunks { checksum, .. } => *checksum,
        }
    }

    /// Bytes it assembles to beyond what the file holds.
    fn assembled_len(&self) -> usize {
        match self {
            Self::Document(_) | Self::Whole { .. } => 0,
            Self::Chunks { length, .. } => *length,
        }
    }

    /// The snapshot bytes, checked against their checksum.
    fn assemble(&self, chunks: &[&[u8]]) -> Result<std::borrow::Cow<'a, [u8]>, PersistenceError> {
        let (encoded, checksum) = match *self {
            Self::Document(bytes) => return Ok(std::borrow::Cow::Borrowed(bytes)),
            Self::Whole { bytes, checksum } => (std::borrow::Cow::Borrowed(bytes), checksum),
            Self::Chunks {
                length,
                checksum,
                numbers,
            } => {
                let parts = numbers.chunks_exact(4).map(|number| {
                    let number = u32::from_le_bytes(number.try_into().expect("four bytes"));
                    chunks
                        .get(number as usize)
                        .copied()
                        .ok_or(PersistenceError::InvalidRevisionHistory)
                });
                // The chunks are summed before anything is allocated and the buffer
                // is sized from that sum, never from the length the file names; the
                // sum stops as soon as it passes that length, which counted against
                // the history budget when the history was read.
                let mut total = 0usize;
                for part in parts.clone() {
                    total = total
                        .checked_add(part?.len())
                        .filter(|total| *total <= length)
                        .ok_or(PersistenceError::InvalidRevisionHistory)?;
                }
                if total != length {
                    return Err(PersistenceError::HistoryChecksumMismatch);
                }
                let mut encoded = Vec::with_capacity(total);
                for part in parts {
                    encoded.extend_from_slice(part?);
                }
                (std::borrow::Cow::Owned(encoded), checksum)
            }
        };
        if crate::graph::sha256_bytes(&encoded) != checksum {
            return Err(PersistenceError::HistoryChecksumMismatch);
        }
        Ok(encoded)
    }
}

/// One history revision as an editable snapshot with the source digest it was
/// loaded with. A current-format revision keeps the bytes it was read from as its
/// encoding, so the next save does not encode it again.
fn load_history_snapshot(
    encoded: &[u8],
    container_data: &ContainerData,
    migrated_digests: &mut BTreeMap<String, String>,
) -> Result<(ValidatedSnapshot, String), PersistenceError> {
    let loaded = load_document(encoded, container_data.clone(), migrated_digests)?;
    let source_schema = loaded.source_schema();
    let source_digest = loaded.audit().source_canonical_digest.clone();
    // A snapshot that loads only for review cannot be a revision to return to.
    let Ok(store) = loaded.into_editable() else {
        return Err(PersistenceError::InvalidRevisionHistory);
    };
    let snapshot = store.current();
    if source_schema == CURRENT_SCHEMA {
        match snapshot_encoding::compare(&snapshot, encoded)? {
            snapshot_encoding::Encoding::Canonical(canonical) => {
                if let Ok(mut cache) = snapshot.product().exact_graphs.encoded.lock() {
                    cache.insert(
                        snapshot.revision_id(),
                        Arc::new(crate::document::EncodedSnapshot {
                            sha256: crate::graph::sha256_bytes(&canonical),
                            bytes: canonical,
                            chunks: std::sync::OnceLock::new(),
                        }),
                    );
                }
            }
            snapshot_encoding::Encoding::Equivalent => {}
            snapshot_encoding::Encoding::Different => {
                return Err(PersistenceError::InvalidRevisionHistory);
            }
        }
    }
    Ok((store.validated_current(), source_digest))
}

/// `load` of every record, spread over the available cores, in record order.
fn load_in_parallel<T: Send>(
    records: &[HistoryRecord<'_>],
    load: impl Fn(usize, &HistoryRecord<'_>) -> Result<T, PersistenceError> + Sync,
) -> Result<Vec<T>, PersistenceError> {
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(records.len())
        .max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut loaded = std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut loaded = Vec::new();
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(record) = records.get(index) else {
                            return loaded;
                        };
                        let result = load(index, record);
                        let failed = result.is_err();
                        loaded.push((index, result));
                        if failed {
                            next.store(records.len(), std::sync::atomic::Ordering::Relaxed);
                            return loaded;
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a history load thread panicked"))
            .collect::<Vec<_>>()
    });
    loaded.sort_by_key(|(index, _)| *index);
    if let Some(error) = loaded.iter().position(|(_, result)| result.is_err()) {
        return Err(loaded.swap_remove(error).1.err().expect("an error"));
    }
    if loaded.len() != records.len() {
        return Err(PersistenceError::InvalidRevisionHistory);
    }
    Ok(loaded
        .into_iter()
        .map(|(_, result)| result.expect("checked"))
        .collect())
}

/// The encoded snapshot of one history revision: stored whole before format 4, as
/// chunk numbers from format 4, or as `document.bin` for the current revision.
/// Nothing is assembled yet.
fn read_history_snapshot<'a>(
    reader: &mut Reader<'a>,
    schema: u16,
    document_bytes: Option<&'a [u8]>,
) -> Result<HistorySnapshot<'a>, PersistenceError> {
    if schema >= 4 {
        match reader.u8()? {
            HISTORY_SNAPSHOT_IS_DOCUMENT => {
                return document_bytes
                    .map(HistorySnapshot::Document)
                    .ok_or(PersistenceError::InvalidRevisionHistory);
            }
            HISTORY_SNAPSHOT_IN_CHUNKS => {}
            _ => return Err(PersistenceError::InvalidRevisionHistory),
        }
    }
    let length = usize::try_from(reader.u64()?)
        .map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)?;
    let checksum: [u8; 32] = reader
        .take(32)?
        .try_into()
        .map_err(|_: std::array::TryFromSliceError| PersistenceError::Truncated)?;
    if schema >= 4 {
        return Ok(HistorySnapshot::Chunks {
            length,
            checksum,
            numbers: chunk_numbers(reader)?,
        });
    }
    Ok(HistorySnapshot::Whole {
        bytes: reader.take(length)?,
        checksum,
    })
}

/// Length, SHA-256 and chunk numbers of at most `max_length` bytes.
fn read_chunk_list<'a>(
    reader: &mut Reader<'a>,
    max_length: usize,
) -> Result<(usize, [u8; 32], &'a [u8]), PersistenceError> {
    let length = usize::try_from(reader.u64()?)
        .map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)?;
    if length > max_length {
        return Err(PersistenceError::ResourceLimit);
    }
    let checksum: [u8; 32] = reader
        .take(32)?
        .try_into()
        .map_err(|_: std::array::TryFromSliceError| PersistenceError::Truncated)?;
    Ok((length, checksum, chunk_numbers(reader)?))
}

/// The chunk numbers (`u32` each) of a chunk list.
fn chunk_numbers<'a>(reader: &mut Reader<'a>) -> Result<&'a [u8], PersistenceError> {
    let count = usize::try_from(reader.count_with_limit(MAX_COLLECTION_ITEMS)?)
        .map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)?;
    reader.take(count * 4)
}

/// A revision's rule program from its JSON.
fn decode_rule_program(
    json: &[u8],
) -> Result<crate::document::RuleProgramSource, PersistenceError> {
    /// Every field a program has ever stored; its other maps are keyed by
    /// parameter and variant names.
    const FIELDS: [&str; 4] = ["file_name", "source", "overrides", "variants"];
    // Save refuses a larger program, so opening one would leave a window that
    // can never write a checkpoint.
    if json.len() > crate::document::MAX_RULE_PROGRAM_BYTES {
        return Err(PersistenceError::ResourceLimit);
    }
    let invalid =
        |error: serde_json::Error| PersistenceError::payload(PayloadSection::RuleProgram, error);
    let stored: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(json).map_err(invalid)?;
    // Serde skips a field it does not know, so the program would open without
    // it and the next Save would drop what a newer version wrote.
    let unknown: Vec<String> = stored
        .keys()
        .filter(|key| !FIELDS.contains(&key.as_str()))
        .map(|key| format!("program.{key}"))
        .collect();
    if !unknown.is_empty() {
        return Err(PersistenceError::UnknownFields(unknown));
    }
    let source: crate::document::RuleProgramSource =
        serde_json::from_value(serde_json::Value::Object(stored)).map_err(invalid)?;
    source
        .validate()
        .map_err(PersistenceError::InvalidRuleProgram)?;
    Ok(source)
}

fn load_document(
    bytes: &[u8],
    mut container_data: ContainerData,
    migrated_digests: &mut BTreeMap<String, String>,
) -> Result<LoadOutcome, PersistenceError> {
    #[cfg(feature = "testing")]
    SNAPSHOT_DECODES.set(SNAPSHOT_DECODES.get() + 1);
    let body = bytes
        .strip_prefix(MAGIC.as_slice())
        .ok_or(PersistenceError::InvalidMagic)?;
    let (schema, body) = body
        .split_at_checked(2)
        .ok_or(PersistenceError::Truncated)?;
    let schema = u16::from_le_bytes([schema[0], schema[1]]);
    let (revision_id, mut product, migration_losses, legacy_digest) = if schema > CURRENT_SCHEMA {
        return Err(PersistenceError::UnsupportedSchema(schema));
    } else if schema >= snapshot_codec::FIRST_SNAPSHOT_FORMAT {
        let decoded = snapshot_codec::decode(schema, body)?;
        (
            decoded.revision_id,
            decoded.product,
            Vec::new(),
            decoded.writer_digest,
        )
    } else {
        let decoded = legacy::decode(bytes)?;
        let migrated = crate::document::digest_v3::migrate_stored_digests(
            decoded.product,
            &decoded.old,
            migrated_digests,
            |product| legacy::complete_old_records(product, &decoded.old),
        )
        .map_err(|error| PersistenceError::payload(PayloadSection::LegacyDigests, error))?;
        (
            decoded.revision_id,
            migrated.product,
            decoded.migration_losses,
            Some(migrated.source_digest),
        )
    };
    if schema < 100 {
        snapshot_codec::rename_format_99_role_categories(&mut product);
    }

    let override_health = product
        .overrides
        .values()
        .map(|value| OverrideHealthAudit {
            override_id: value.id,
            stored: value.health.clone(),
            audited: crate::graph::resolve_derived_identity(
                &product.evaluator_nodes,
                &value.target,
            ),
        })
        .collect::<Vec<_>>();
    let review_required = !migration_losses.is_empty()
        || override_health.iter().any(|entry| {
            entry.audited != SlotResolution::Resolved || entry.audited != entry.stored
        });
    let document = DocumentStore::from_product(revision_id, product)?;
    let feature_parameter_freshness = document
        .current()
        .audit_feature_parameter_freshness(&EvaluationIdentity::default())?;
    let audit = LoadAudit {
        source_schema: schema,
        source_canonical_digest: legacy_digest
            .unwrap_or_else(|| document.current().canonical_digest()),
        migration_losses,
        override_health,
        feature_parameter_freshness,
        unknown_extensions: container_data
            .extensions()
            .map(|entry| ExtensionAudit {
                namespace: entry.namespace().to_owned(),
                path: entry.path().to_owned(),
                required: entry.required(),
            })
            .collect(),
        notices: Vec::new(),
    };
    let loaded_snapshot = document.current();
    let imported_sources = loaded_snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some((
                spec.source_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
                spec.source_byte_len,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (hash, byte_len) in imported_sources {
        if !container_data
            .blobs
            .get(&hash)
            .is_some_and(|bytes| bytes.len() as u64 == byte_len)
        {
            return Err(PersistenceError::InvalidBlobHash);
        }
        Arc::make_mut(&mut container_data.imported_source_blobs).insert(hash);
    }
    // An underlay's lines, like an imported source, leave the file with the last
    // revision that names them.
    for hash in loaded_snapshot
        .underlays()
        .map(crate::underlay::Underlay::lines_blob)
    {
        if !container_data.blobs.contains_key(&hash) {
            return Err(PersistenceError::InvalidBlobHash);
        }
        Arc::make_mut(&mut container_data.imported_source_blobs).insert(hash);
    }
    // A stored face or edge reference must still name the body its producer
    // builds. Evaluator digests may differ: older files were evaluated by other
    // code paths, and the next evaluation refreshes the evidence. Each producer's
    // graph is compiled once for all the references it carries.
    let mut graphs = BTreeMap::new();
    for reference in loaded_snapshot.exact_reference_evidence() {
        let graph = graphs
            .entry((reference.definition_id, reference.producer_feature_id))
            .or_insert_with(|| {
                #[cfg(test)]
                tests::EVIDENCE_GRAPH_COMPILES.with(|count| count.set(count.get() + 1));
                crate::exact_brep_graph::ExactBRepGraph::from_snapshot(
                    &loaded_snapshot,
                    reference.definition_id,
                    reference.producer_feature_id,
                )
                .ok()
            });
        let names_current_body = graph
            .as_ref()
            .is_some_and(|graph| graph.names_durable_reference(reference));
        if !names_current_body {
            return Err(PersistenceError::InvalidExactReference);
        }
    }
    if review_required || container_data.requires_unknown_extension() {
        Ok(LoadOutcome::ReviewOnly(ReviewCandidate {
            snapshot: document.current(),
            audit: Box::new(audit),
            container_data: Box::new(container_data),
            program: None,
        }))
    } else {
        Ok(LoadOutcome::Editable {
            document,
            audit,
            container_data,
        })
    }
}

fn validate_namespace(namespace: &str) -> Result<(), PersistenceError> {
    if namespace.is_empty()
        || namespace.len() > MAX_CONTAINER_PATH_BYTES
        || !namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        return Err(PersistenceError::InvalidContainerPath(namespace.to_owned()));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), PersistenceError> {
    if path.is_empty()
        || path.len() > MAX_CONTAINER_PATH_BYTES
        || path.contains('\\')
        || path.contains('\0')
        || path.starts_with('/')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(PersistenceError::InvalidContainerPath(path.to_owned()));
    }
    Ok(())
}

fn validate_container_path(path: &str) -> Result<(), PersistenceError> {
    validate_relative_path(path)?;
    if path.contains(':') {
        return Err(PersistenceError::InvalidContainerPath(path.to_owned()));
    }
    Ok(())
}

#[derive(Debug)]
pub enum FilePersistenceError {
    Io(io::Error),
    Format(PersistenceError),
    ExternalConflict,
    /// The work-recovery checkpoint's path holds something other than a file, which a
    /// session neither reads nor replaces.
    NotARegularFile(PathBuf),
    /// Every set-aside name next to this work-recovery checkpoint is taken.
    SetAsideNamesTaken(PathBuf),
    /// The file did not open (`primary`), and neither did its `.recovery` backup.
    BackupUnusable {
        primary: Box<FilePersistenceError>,
        backup_path: PathBuf,
        backup: Box<FilePersistenceError>,
    },
    /// The complete file is visible, but directory durability could not be confirmed.
    Published {
        path: PathBuf,
        identity: FileIdentity,
        error: io::Error,
    },
}

impl FilePersistenceError {
    pub fn published_identity(&self, path: &Path) -> Option<FileIdentity> {
        match self {
            Self::Published {
                path: published,
                identity,
                ..
            } if published == path => Some(*identity),
            _ => None,
        }
    }
}

impl fmt::Display for FilePersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Published { path, error, .. } => write!(
                formatter,
                "{} was published, but its durability is unconfirmed: {error}; keep the current state and retry saving",
                path.display()
            ),
            Self::Format(error) => error.fmt(formatter),
            Self::ExternalConflict => {
                formatter.write_str("the document changed outside this session")
            }
            Self::NotARegularFile(path) => {
                write!(formatter, "{} is not a regular file", path.display())
            }
            Self::SetAsideNamesTaken(path) => write!(
                formatter,
                "a thousand set-aside copies of {} already exist",
                path.display()
            ),
            Self::BackupUnusable {
                primary,
                backup_path,
                backup,
            } => write!(
                formatter,
                "{primary}; its backup {} did not open either: {backup}",
                backup_path.display()
            ),
        }
    }
}

impl std::error::Error for FilePersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) | Self::Published { error, .. } => Some(error),
            Self::Format(error) => Some(error),
            Self::BackupUnusable { primary, .. } => Some(primary.as_ref()),
            Self::ExternalConflict | Self::NotARegularFile(_) | Self::SetAsideNamesTaken(_) => None,
        }
    }
}
impl From<io::Error> for FilePersistenceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Why a semantic migration cannot be confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationBlocker {
    UnknownExtension,
    NoSemanticLoss,
    UnresolvedOverride,
    UnsupportedLoss,
    NotAParameterDimension,
}

impl fmt::Display for MigrationBlocker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownExtension => "candidate requires an unknown extension",
            Self::NoSemanticLoss => "candidate has no reported semantic loss",
            Self::UnresolvedOverride => "candidate has unresolved override identity",
            Self::UnsupportedLoss => "candidate contains an unsupported migration loss",
            Self::NotAParameterDimension => "migration target is not a parameter dimension",
        })
    }
}

/// A file to remove changed after it was claimed: it is put back, and the claimed
/// entry stays at `claimed_path`, so nothing is lost. The `io::Error` a cleanup returns
/// carries this.
#[derive(Debug)]
pub struct CleanupRaced {
    pub claimed_path: PathBuf,
    pub error: io::Error,
}

impl fmt::Display for CleanupRaced {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cleanup target changed concurrently; preserved claimed entry at {}: {}",
            self.claimed_path.display(),
            self.error
        )
    }
}

impl std::error::Error for CleanupRaced {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PersistenceError {
    /// A pre-serde document failed the frozen legacy reader.
    Legacy(LegacyError),
    MigrationNotConfirmable(MigrationBlocker),
    Truncated,
    InvalidMagic,
    InvalidContainerMagic,
    UnsupportedSchema(u16),
    UnsupportedContainerSchema(u16),
    UnsupportedHistorySchema(u16),
    InvalidHistoryMagic,
    InvalidRevisionHistory,
    HistoryChecksumMismatch,
    MissingDocumentEntry,
    DocumentEntryNotRequired,
    HistoryEntryNotRequired,
    ProgramEntryNotRequired,
    DuplicateContainerEntry,
    InvalidContainerPath(String),
    UnsupportedContainerEntry(String),
    ContainerChecksumMismatch(String),
    InvalidBlobHash,
    InvalidUtf8(std::str::Utf8Error),
    LengthOverflow,
    TrailingBytes,
    InvalidBoolean(u8),
    InvalidExactReference,
    ChecksumMismatch,
    /// A part of the document did not decode: which part, and why.
    InvalidPayload {
        section: PayloadSection,
        error: PayloadError,
    },
    /// The document holds fields this version does not know (by path), which opening it
    /// would drop and the next Save would lose.
    UnknownFields(Vec<String>),
    InvalidRuleProgram(crate::document::RuleProgramError),
    ResourceLimit,
    /// A container entry, or the whole file, is larger than a document may be.
    TooLarge {
        entry: String,
        bytes: u64,
        limit: u64,
    },
    LegacyFeatureRequiresMigration {
        feature_id: FeatureId,
        kind: LegacyFeatureKind,
    },
    InvalidCanonicalData(CanonicalError),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy(error) => write!(formatter, "legacy document: {error}"),
            Self::MigrationNotConfirmable(reason) => {
                write!(
                    formatter,
                    "semantic migration cannot be confirmed: {reason}"
                )
            }
            Self::Truncated => formatter.write_str("document is truncated"),
            Self::InvalidMagic => formatter.write_str("document magic is invalid"),
            Self::InvalidContainerMagic => formatter.write_str("container magic is invalid"),
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "document schema {schema} is unsupported")
            }
            Self::UnsupportedContainerSchema(schema) => {
                write!(formatter, "container schema {schema} is unsupported")
            }
            Self::UnsupportedHistorySchema(schema) => {
                write!(formatter, "revision history schema {schema} is unsupported")
            }
            Self::InvalidHistoryMagic => formatter.write_str("revision history magic is invalid"),
            Self::InvalidRevisionHistory => formatter.write_str("revision history is invalid"),
            Self::HistoryChecksumMismatch => {
                formatter.write_str("revision history snapshot checksum does not match")
            }
            Self::MissingDocumentEntry => formatter.write_str("container has no document.bin"),
            Self::DocumentEntryNotRequired => {
                formatter.write_str("container document.bin must be required")
            }
            Self::HistoryEntryNotRequired => {
                formatter.write_str("container history.bin must be required")
            }
            Self::ProgramEntryNotRequired => {
                formatter.write_str("container program.json must be required")
            }
            Self::DuplicateContainerEntry => formatter.write_str("container repeats an entry"),
            Self::InvalidContainerPath(path) => {
                write!(formatter, "container path {path:?} is unsafe")
            }
            Self::UnsupportedContainerEntry(path) => {
                write!(formatter, "container entry {path:?} is unsupported")
            }
            Self::ContainerChecksumMismatch(path) => {
                write!(
                    formatter,
                    "container entry {path:?} checksum does not match"
                )
            }
            Self::InvalidBlobHash => {
                formatter.write_str("container blob content hash does not match its path")
            }
            Self::InvalidUtf8(error) => write!(formatter, "document string is not UTF-8: {error}"),
            Self::LengthOverflow => formatter.write_str("document length exceeds this platform"),
            Self::TrailingBytes => formatter.write_str("document has trailing bytes"),
            Self::InvalidBoolean(value) => write!(formatter, "document boolean {value} is invalid"),
            Self::InvalidExactReference => {
                formatter.write_str("exact reference evidence is invalid")
            }
            Self::ChecksumMismatch => formatter.write_str("document checksum does not match"),
            Self::InvalidPayload { section, error } => {
                write!(formatter, "{section} of the document {error}")
            }
            Self::UnknownFields(paths) => write!(
                formatter,
                "the document holds data this version of Kečup does not know ({}); it was \
                 probably saved by a newer version, and opening it here would lose that data",
                paths.join(", ")
            ),
            Self::InvalidRuleProgram(error) => {
                write!(formatter, "the document's program is invalid: {error}")
            }
            Self::ResourceLimit => formatter.write_str("document exceeds a resource limit"),
            Self::TooLarge {
                entry,
                bytes,
                limit,
            } => {
                let megabytes = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
                write!(
                    formatter,
                    "{entry} would be at least {:.1} MB ({bytes} bytes); the limit is {:.1} MB",
                    megabytes(*bytes),
                    megabytes(*limit)
                )
            }
            Self::LegacyFeatureRequiresMigration { feature_id, kind } => {
                let kind = match kind {
                    LegacyFeatureKind::RoleStringShell => "role-string Shell",
                };
                write!(
                    formatter,
                    "legacy feature {} ({kind}) requires explicit migration",
                    feature_id.0
                )
            }
            Self::InvalidCanonicalData(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidUtf8(error) => Some(error),
            Self::InvalidCanonicalData(error) => Some(error),
            _ => None,
        }
    }
}
impl PersistenceError {
    pub(crate) fn payload(section: PayloadSection, error: impl Into<PayloadError>) -> Self {
        Self::InvalidPayload {
            section,
            error: error.into(),
        }
    }
}

impl From<ketchup_geometry::dimension::DimensionError> for PersistenceError {
    fn from(error: ketchup_geometry::dimension::DimensionError) -> Self {
        CanonicalError::from(error).into()
    }
}

impl From<CanonicalError> for PersistenceError {
    fn from(error: CanonicalError) -> Self {
        Self::InvalidCanonicalData(error)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
    collection_items: u64,
    item_budget: u32,
    string_bytes: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self::with_item_budget(bytes, MAX_COLLECTION_ITEMS)
    }
    const fn with_item_budget(bytes: &'a [u8], item_budget: u32) -> Self {
        Self {
            bytes,
            cursor: 0,
            collection_items: 0,
            item_budget,
            string_bytes: 0,
        }
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], PersistenceError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(PersistenceError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(PersistenceError::Truncated)?;
        self.cursor = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, PersistenceError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, PersistenceError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(
            |_: std::array::TryFromSliceError| PersistenceError::Truncated,
        )?))
    }
    fn u32(&mut self) -> Result<u32, PersistenceError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(
            |_: std::array::TryFromSliceError| PersistenceError::Truncated,
        )?))
    }
    fn u64(&mut self) -> Result<u64, PersistenceError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(
            |_: std::array::TryFromSliceError| PersistenceError::Truncated,
        )?))
    }
    fn count_with_limit(&mut self, limit: u32) -> Result<u32, PersistenceError> {
        let value = self.u32()?;
        self.collection_items = self
            .collection_items
            .checked_add(u64::from(value))
            .ok_or(PersistenceError::ResourceLimit)?;
        if value > limit || self.collection_items > u64::from(self.item_budget) {
            Err(PersistenceError::ResourceLimit)
        } else {
            Ok(value)
        }
    }
    fn string(&mut self) -> Result<String, PersistenceError> {
        // The running string budget below bounds each length.
        let length = usize::try_from(self.count_with_limit(u32::MAX)?)
            .map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)?;
        self.string_bytes = self
            .string_bytes
            .checked_add(length)
            .ok_or(PersistenceError::ResourceLimit)?;
        if self.string_bytes > MAX_STRING_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
        let value =
            std::str::from_utf8(self.take(length)?).map_err(PersistenceError::InvalidUtf8)?;
        // The running string budget above bounds this allocation.
        Ok(value.to_owned())
    }
    fn boolean(&mut self) -> Result<bool, PersistenceError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(PersistenceError::InvalidBoolean(value)),
        }
    }
    const fn is_finished(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

fn push_u8(bytes: &mut Vec<u8>, value: u8) {
    bytes.push(value);
}
fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_string(bytes: &mut Vec<u8>, value: &str) -> Result<(), PersistenceError> {
    push_u32(bytes, u32_count(value.len())?);
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DefinitionId;

    thread_local! {
        pub(super) static EVIDENCE_GRAPH_COMPILES: std::cell::Cell<usize> =
            const { std::cell::Cell::new(0) };
        pub(super) static HISTORY_DECODES: std::cell::Cell<usize> =
            const { std::cell::Cell::new(0) };
    }

    /// PS-1: a budget a history fits is one it saves and opens with; one byte
    /// less refuses both, so no saved history is dropped on opening.
    #[test]
    fn saving_and_opening_a_history_keep_the_same_assembled_budget() {
        use crate::document::{CanonicalCommand, CommandBatch, NodeId};
        use ketchup_geometry::dimension::Dimension;
        let mut store = DocumentStore::new();
        for id in 1..=3_u64 {
            store
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::CreateEvaluatorNode {
                        id: NodeId(id),
                        name: format!("parameter-{id}"),
                        dimension: Dimension::new(id.to_string(), id as f64).unwrap(),
                        dependencies: vec![],
                    },
                ]))
                .unwrap();
        }
        let revisions = store.revision_history().collect::<Vec<_>>();
        let cursor = store.history_cursor();
        let document_bin = encoded_snapshot(&store.current());
        let fits = revisions
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != cursor)
            .map(|(_, revision)| encoded_snapshot(revision.snapshot()).bytes.len() as u64)
            .sum::<u64>();
        let budget = |snapshots| HistoryBudget {
            snapshots,
            ..HISTORY_BUDGET
        };
        let encode = |snapshots| {
            encode_history(
                &revisions,
                cursor,
                store.next_revision_id(),
                &document_bin,
                budget(snapshots),
            )
        };
        let history = encode(fits).unwrap();
        assert!(matches!(
            encode(fits - 1),
            Err(PersistenceError::TooLarge { bytes, limit, .. }) if bytes == fits && limit == fits - 1
        ));

        let outcome = load(&save(&store.current())).unwrap();
        let decode = |snapshots| {
            decode_revision_history(
                &history,
                &document_bin.bytes,
                &outcome,
                &ContainerData::default(),
                budget(snapshots),
            )
        };
        assert_eq!(decode(fits).unwrap().revision_count(), revisions.len());
        assert!(matches!(
            decode(fits - 1),
            Err(PersistenceError::TooLarge { bytes, .. }) if bytes == fits
        ));
    }

    /// PS-9: Save keeps the item budget the reader of a history keeps, so a
    /// history that saves also opens and one past it is refused on Save (which
    /// keeps the current revision then).
    #[test]
    fn saving_and_opening_a_history_keep_the_same_item_limit() {
        use crate::document::{CanonicalCommand, CommandBatch, NodeId, RuleProgramSource};
        use ketchup_geometry::dimension::Dimension;
        let mut store = DocumentStore::new();
        for id in 1..=3_u64 {
            store
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::CreateEvaluatorNode {
                        id: NodeId(id),
                        name: format!("parameter-{id}"),
                        dimension: Dimension::new(id.to_string(), id as f64).unwrap(),
                        dependencies: vec![],
                    },
                ]))
                .unwrap();
        }
        for name in ["A", "B"] {
            let program = RuleProgramSource {
                file_name: "model.star".into(),
                source: format!("# {name}\n{}", "x = 1\n".repeat(20_000)),
                overrides: BTreeMap::new(),
                variants: BTreeMap::new(),
            };
            if name == "A" {
                store.bind_rule_program(program).unwrap();
            } else {
                store.replace_rule_program_source(program).unwrap();
            }
        }
        let revisions = store.revision_history().collect::<Vec<_>>();
        let cursor = store.history_cursor();
        let document_bin = encoded_snapshot(&store.current());
        let outcome = load(&save(&store.current())).unwrap();
        let encode = |budget| {
            encode_history(
                &revisions,
                cursor,
                store.next_revision_id(),
                &document_bin,
                budget,
            )
        };
        let decode = |history: &[u8], budget| {
            decode_revision_history(
                history,
                &document_bin.bytes,
                &outcome,
                &ContainerData::default(),
                budget,
            )
        };

        let items = |items| HistoryBudget {
            items,
            ..HISTORY_BUDGET
        };
        // The fewest items Save accepts are the fewest opening accepts.
        let (mut refused, mut fits) = (0, MAX_COLLECTION_ITEMS);
        while fits - refused > 1 {
            let middle = refused + (fits - refused) / 2;
            if encode(items(middle)).is_ok() {
                fits = middle;
            } else {
                refused = middle;
            }
        }
        let history = encode(items(fits)).unwrap();
        assert_eq!(
            decode(&history, items(fits)).unwrap().revision_count(),
            revisions.len()
        );
        assert!(matches!(
            encode(items(fits - 1)),
            Err(PersistenceError::ResourceLimit)
        ));
        assert!(matches!(
            decode(&history, items(fits - 1)),
            Err(PersistenceError::ResourceLimit)
        ));
    }

    /// An earlier revision's length is the file's word: a short list of chunk
    /// numbers can repeat one chunk far past the revision budget. Opening sums the
    /// chunks before it allocates, so a length they do not fill is refused without
    /// reserving it, and one they fill counts against the budget first.
    #[test]
    fn a_revision_length_its_chunks_do_not_fill_is_refused_before_it_is_allocated() {
        let store = DocumentStore::new();
        let document_bin = encoded_snapshot(&store.current());
        let outcome = load(&save(&store.current())).unwrap();
        let chunk = document_bin.bytes.len() as u64;
        let checksum = crate::graph::sha256_bytes(&document_bin.bytes);
        // Revision 1 names `repeats` copies of document.bin as `length` bytes;
        // revision 2 is document.bin. Both are refused before their ids matter.
        let history = |length: u64, repeats: u32| {
            let mut history = HISTORY_MAGIC.to_vec();
            push_u16(&mut history, 4);
            push_u32(&mut history, 2);
            push_u32(&mut history, 1);
            push_u64(&mut history, 3);
            push_u32(&mut history, 1);
            history.push(HISTORY_CHUNK_IN_DOCUMENT);
            push_u32(&mut history, 0);
            push_u32(&mut history, u32::try_from(chunk).unwrap());
            for (index, revision_id) in [1_u64, 2].into_iter().enumerate() {
                push_u64(&mut history, revision_id);
                push_u32(&mut history, 0); // no batch digest
                if index == 0 {
                    history.push(0); // initial
                } else {
                    history.extend_from_slice(&[1, 0]); // the manual client
                }
                history.extend_from_slice(&[0, 0]); // no checkpoint, no program
                if index == 1 {
                    history.push(HISTORY_SNAPSHOT_IS_DOCUMENT);
                } else {
                    history.push(HISTORY_SNAPSHOT_IN_CHUNKS);
                    push_u64(&mut history, length);
                    history.extend_from_slice(&checksum);
                    push_u32(&mut history, repeats);
                    for _ in 0..repeats {
                        push_u32(&mut history, 0);
                    }
                }
            }
            history
        };
        let decode = |history: &[u8], snapshots| {
            decode_revision_history(
                history,
                &document_bin.bytes,
                &outcome,
                &ContainerData::default(),
                HistoryBudget {
                    snapshots,
                    ..HISTORY_BUDGET
                },
            )
        };
        let repeats = 100_000;
        let named = chunk * u64::from(repeats);

        // Filled by its chunks but past the budget: refused on the length alone.
        assert!(matches!(
            decode(&history(named, repeats), named - 1),
            Err(PersistenceError::TooLarge { bytes, .. }) if bytes == named
        ));
        // A petabyte with no budget to stop it: reserving it would abort the test.
        assert_eq!(
            decode(&history(1 << 50, repeats), u64::MAX).err(),
            Some(PersistenceError::HistoryChecksumMismatch)
        );
        // Fewer bytes than the chunks hold: refused while summing them.
        assert_eq!(
            decode(&history(chunk, repeats), u64::MAX).err(),
            Some(PersistenceError::InvalidRevisionHistory)
        );
    }

    /// PS-10: a chunk range past what a `u32` holds is refused, not written as
    /// the truncated range of other bytes, and leaves the table as it was.
    #[test]
    fn a_history_chunk_range_past_u32_is_refused_not_truncated() {
        let past = usize::try_from(u64::from(u32::MAX) + 1).unwrap();
        let mut table = ChunkTable::default();
        for range in [past..past + 16, 0..past] {
            let chunk = history_chunks::Chunk {
                range,
                sha256: [7; 32],
            };
            assert!(matches!(
                table.add_document_chunk(&chunk),
                Err(PersistenceError::LengthOverflow)
            ));
        }
        assert!(table.numbers.is_empty() && table.bytes.is_empty());
        let chunk = history_chunks::Chunk {
            range: 16..32,
            sha256: [7; 32],
        };
        table.add_document_chunk(&chunk).unwrap();
        assert_eq!(table.numbers.get(&[7; 32]), Some(&0));
        assert_eq!(table.bytes[1..], [16, 0, 0, 0, 16, 0, 0, 0]);
    }

    /// PS-6: Save checks the bytes it writes and the file it replaces without
    /// decoding an Undo history; the saved file opens without Undo steps.
    #[test]
    fn saving_over_a_file_decodes_no_history() {
        use crate::document::{CanonicalCommand, CommandBatch, NodeId};
        use ketchup_geometry::dimension::Dimension;
        let mut store = DocumentStore::new();
        for id in 1..=3_u64 {
            store
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::CreateEvaluatorNode {
                        id: NodeId(id),
                        name: format!("parameter-{id}"),
                        dimension: Dimension::new(id.to_string(), id as f64).unwrap(),
                        dependencies: vec![],
                    },
                ]))
                .unwrap();
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.ketchup");
        let container = ContainerData::default();
        let prepared = prepare_document(&store, &container).unwrap();
        publish_document(&path, &prepared, Publish::IfAbsent).unwrap();
        HISTORY_DECODES.with(|count| count.set(0));
        publish_document(&path, &prepared, Publish::IfUnchanged(prepared.identity())).unwrap();
        assert_eq!(HISTORY_DECODES.with(std::cell::Cell::get), 0);
        assert!(load(&fs::read(recovery_path(&path)).unwrap()).is_ok());
        let opened = load_file(&path).unwrap();
        let opened = opened.editable_document().expect("editable");
        assert_eq!(opened.revision_count(), 1);
    }

    /// PS-1: a file that opens only for migration review keeps the program it
    /// stores beside the document through the migration and its save.
    #[test]
    fn a_migrated_document_keeps_the_program_the_file_stores() {
        use crate::document::{EncodedSnapshot, NodeId, RuleProgramSource};
        let mut legacy = b"KETCHUPDOC".to_vec();
        legacy.extend_from_slice(&0_u16.to_le_bytes());
        legacy.extend_from_slice(&7_u64.to_le_bytes());
        legacy.extend_from_slice(&1_u32.to_le_bytes());
        legacy.extend_from_slice(&42_u64.to_le_bytes());
        legacy.extend_from_slice(&1_u32.to_le_bytes());
        legacy.push(b'x');
        legacy.extend_from_slice(&3.5_f64.to_bits().to_le_bytes());
        legacy.extend_from_slice(&0_u32.to_le_bytes());
        let program = RuleProgramSource {
            file_name: "model.star".into(),
            source: "x = 3.5\n".into(),
            overrides: BTreeMap::new(),
            variants: BTreeMap::new(),
        };
        let file = save_container_entries(
            &EncodedSnapshot {
                sha256: crate::graph::sha256_bytes(&legacy),
                bytes: legacy,
                chunks: std::sync::OnceLock::new(),
            },
            &ContainerData::default(),
            BTreeSet::new(),
            None,
            Some(serde_json::to_vec(&program).unwrap()),
        )
        .unwrap();

        let loaded = load(&file).unwrap();
        let (document, container) = loaded
            .review_candidate()
            .expect("a legacy dimension needs review")
            .confirm_semantic_migration()
            .unwrap()
            .into_parts();
        assert_eq!(document.current_rule_program(), Some(&program));
        let saved = save_document_store_current_snapshot(&document, &container).unwrap();
        let reopened = load(&saved).unwrap();
        let reopened = reopened
            .editable_document()
            .expect("migrated file is editable");
        assert_eq!(reopened.current_rule_program(), Some(&program));
        assert!(reopened.current().evaluator_node(NodeId(42)).is_some());
    }

    /// PS-5: revisions that share one product (a program source edit, a rollback)
    /// are each encoded once, not again on every history write.
    #[test]
    fn revisions_sharing_a_product_are_each_encoded_once() {
        use crate::document::{CanonicalCommand, CommandBatch, NodeId, RuleProgramSource};
        use ketchup_geometry::dimension::Dimension;
        let program = |name: &str| RuleProgramSource {
            file_name: "model.star".into(),
            source: format!("# {name}\n"),
            overrides: BTreeMap::new(),
            variants: BTreeMap::new(),
        };
        let mut store = DocumentStore::new();
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateEvaluatorNode {
                    id: NodeId(1),
                    name: "parameter".into(),
                    dimension: Dimension::new("1", 1.0).unwrap(),
                    dependencies: vec![],
                },
            ]))
            .unwrap();
        store.bind_rule_program(program("0")).unwrap();
        for edit in 1..=4 {
            store
                .replace_rule_program_source(program(&edit.to_string()))
                .unwrap();
        }
        let revisions = store.revision_history().skip(1).collect::<Vec<_>>();
        assert_eq!(revisions.len(), 5);
        assert!(
            revisions.windows(2).all(|pair| std::ptr::eq(
                pair[0].snapshot().product(),
                pair[1].snapshot().product()
            ))
        );
        let first = revisions
            .iter()
            .map(|revision| encoded_snapshot(revision.snapshot()))
            .collect::<Vec<_>>();
        for (revision, encoded) in revisions.iter().zip(&first) {
            assert_eq!(encoded.bytes, save(revision.snapshot()));
            assert!(
                Arc::ptr_eq(encoded, &encoded_snapshot(revision.snapshot())),
                "revision {} encoded again",
                revision.id()
            );
        }
    }

    /// PS-2 review 52: programs that chunks of a small `history.bin` name are held
    /// once each, not once per revision, and their distinct bytes are within a
    /// budget Save and opening keep alike.
    #[test]
    fn opening_a_history_holds_each_distinct_program_once() {
        use crate::document::{
            CanonicalCommand, CommandBatch, NodeId, OccurrenceId, RuleProgramSource,
        };
        use ketchup_geometry::dimension::Dimension;
        let program = |name: &str| RuleProgramSource {
            file_name: "model.star".into(),
            source: format!("# {name}\n{}", "x = 1\n".repeat(40_000)),
            overrides: BTreeMap::new(),
            variants: BTreeMap::new(),
        };
        let (a, b) = (program("A"), program("B"));
        let mut store = DocumentStore::new();
        store
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateEvaluatorNode {
                    id: NodeId(1),
                    name: "parameter".into(),
                    dimension: Dimension::new("1", 1.0).unwrap(),
                    dependencies: vec![],
                },
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "part".into(),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(1),
                    name: "part".into(),
                    transform: crate::document::Transform::identity(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                },
            ]))
            .unwrap();
        store.bind_rule_program(a.clone()).unwrap();
        // View-only edits keep the program: written as "same as the previous".
        for edit in 0..8 {
            store
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetOccurrenceVisibility {
                        id: OccurrenceId(2),
                        visible: edit % 2 == 1,
                    },
                ]))
                .unwrap();
        }
        for edit in 0..20 {
            let next = if edit % 2 == 0 { &b } else { &a };
            store.replace_rule_program_source(next.clone()).unwrap();
        }
        let document_bin = encoded_snapshot(&store.current());
        let history = encode_revision_history(&store, &document_bin).unwrap();
        let program_bytes = serde_json::to_vec(&a).unwrap().len();
        // A 240 KB program in almost every revision, two of them distinct.
        assert!(history.len() < 3 * program_bytes, "{}", history.len());

        let outcome = load(&save(&store.current())).unwrap();
        let decode = |history: &[u8], programs| {
            decode_revision_history(
                history,
                &document_bin.bytes,
                &outcome,
                &ContainerData::default(),
                HistoryBudget {
                    programs,
                    ..HISTORY_BUDGET
                },
            )
        };
        let opened = decode(&history, HISTORY_BUDGET.programs).unwrap();
        let held = opened
            .revision_history()
            .filter_map(|revision| revision.rule_program())
            .map(|program| program.source.as_ptr())
            .collect::<BTreeSet<_>>();
        assert_eq!(opened.revision_count(), store.revision_count());
        assert_eq!(held.len(), 2, "programs held, one per distinct program");

        // The distinct programs count against their budget, alike on Save and opening.
        let fits = 2 * u64::try_from(program_bytes).unwrap();
        let revisions = store.revision_history().collect::<Vec<_>>();
        let encode = |programs| {
            encode_history(
                &revisions,
                store.history_cursor(),
                store.next_revision_id(),
                &document_bin,
                HistoryBudget {
                    programs,
                    ..HISTORY_BUDGET
                },
            )
        };
        assert_eq!(encode(fits).unwrap(), history);
        assert!(matches!(
            encode(fits - 1),
            Err(PersistenceError::TooLarge { entry, bytes, .. })
                if entry == HISTORY_PROGRAMS_ENTRY && bytes == fits
        ));
        assert_eq!(
            decode(&history, fits).unwrap().revision_count(),
            store.revision_count()
        );
        assert!(matches!(
            decode(&history, fits - 1),
            Err(PersistenceError::TooLarge { entry, bytes, .. })
                if entry == HISTORY_PROGRAMS_ENTRY && bytes == fits
        ));
    }

    #[test]
    fn opening_compiles_one_graph_per_producer_not_per_face_reference() {
        use crate::exact_product::ExactFaceRole;
        use ketchup_geometry::sketch::{
            FeatureDirection, FeatureExtent, PadOperation, PadProfile, PadSpec, PrincipalPlane,
            SketchEntity, SketchEntityId, SketchSpec, WorkplaneSpec,
        };

        const DEFINITION: DefinitionId = DefinitionId(1);
        let (plane, sketch, pads) = (FeatureId(1), FeatureId(2), [FeatureId(3), FeatureId(4)]);
        let corners = [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        let profile = SketchSpec {
            workplane: plane,
            entities: (0..4)
                .map(|index| SketchEntity::Line {
                    id: SketchEntityId(index as u64 + 1),
                    start_mm: corners[index],
                    end_mm: corners[(index + 1) % 4],
                })
                .collect(),
            constraints: Vec::new(),
        };
        let region = profile.solved_regions().unwrap()[0].id;
        let mut commands = vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Two pads".into(),
            },
            CanonicalCommand::CreateFeature {
                id: plane,
                definition_id: DEFINITION,
                name: "Plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch,
                definition_id: DEFINITION,
                name: "Sketch".into(),
                kind: FeatureKind::Sketch(profile),
            },
        ];
        for (index, pad) in pads.into_iter().enumerate() {
            commands.push(CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: DEFINITION,
                name: format!("Pad {index}"),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion { sketch, region },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(
                        crate::document::Dimension::new(
                            format!("{}", 10 + index),
                            10.0 + index as f64,
                        )
                        .unwrap(),
                    ),
                    operation: PadOperation::NewBody,
                }),
            });
        }
        let mut document = DocumentStore::new();
        document.apply_batch(&CommandBatch::new(commands)).unwrap();
        let roles = [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::LinearSide,
        ];
        for pad in pads {
            let package = crate::testing::box_package(
                &document.current(),
                DEFINITION,
                pad,
                &format!("pad-{}", pad.0),
                &roles,
            )
            .unwrap();
            for role in roles {
                document
                    .register_exact_reference_evidence(package.reference(role).unwrap().clone())
                    .unwrap();
            }
        }
        let bytes = save(&document.current());

        EVIDENCE_GRAPH_COMPILES.with(|count| count.set(0));
        let reopened = load(&bytes).unwrap().snapshot();

        assert_eq!(reopened.exact_reference_evidence().count(), 6);
        assert_eq!(EVIDENCE_GRAPH_COMPILES.with(std::cell::Cell::get), 2);
    }

    #[test]
    fn work_recovery_cleanup_preserves_replacement_after_identity_check() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.ketchup");
        let recovery = work_recovery_path(&path);
        let owned = b"owned recovery";
        let external = b"external replacement";
        fs::write(&recovery, owned).unwrap();

        let removed =
            clear_work_recovery_after_compare(&path, Some(FileIdentity::from_bytes(owned)), || {
                let replacement = directory.path().join("replacement.tmp");
                fs::write(&replacement, external).unwrap();
                fs::rename(replacement, &recovery).unwrap();
            })
            .unwrap();

        assert!(!removed);
        assert_eq!(fs::read(recovery).unwrap(), external);
    }

    #[test]
    fn absent_only_save_preserves_a_concurrently_created_target() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("concurrently-created.ketchup");
        let payload = save(&DocumentStore::new().current());
        let external = b"concurrent external creation";

        assert!(matches!(
            publish_bytes(
                &path,
                &payload,
                Publish::IfAbsent,
                || fs::write(&path, external).unwrap(),
                |_| Ok(()),
            ),
            Err(FilePersistenceError::ExternalConflict)
        ));
        assert_eq!(fs::read(path).unwrap(), external);
    }

    #[test]
    fn absent_only_save_reports_parent_sync_failure_after_complete_publish() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new.ketchup");
        let payload = save(&DocumentStore::new().current());

        let error = publish_bytes(
            &path,
            &payload,
            Publish::IfAbsent,
            || {},
            |_| Err(io::Error::other("injected parent sync failure")),
        )
        .unwrap_err();

        assert!(error.to_string().contains("injected parent sync failure"));
        assert_eq!(fs::read(path).unwrap(), payload);
    }

    #[test]
    fn replacing_save_reports_parent_sync_failure_after_complete_publish() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.ketchup");
        fs::write(&path, b"previous").unwrap();
        let payload = save(&DocumentStore::new().current());

        let error = write_atomic_after_prepare(
            &path,
            &payload,
            true,
            || Ok(()),
            |_| Err(io::Error::other("injected parent sync failure")),
        )
        .unwrap_err();

        assert!(error.to_string().contains("injected parent sync failure"));
        assert_eq!(fs::read(path).unwrap(), payload);
    }

    #[test]
    fn concurrent_conditional_saves_allow_exactly_one_stale_writer() {
        use std::sync::mpsc;
        use std::time::Duration;

        fn document_bytes(name: &str) -> Vec<u8> {
            let mut document = DocumentStore::new();
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::CreateDefinition {
                        id: DefinitionId(1),
                        name: name.into(),
                    },
                ]))
                .unwrap();
            save(&document.current())
        }

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("concurrent-save.ketchup");
        let initial = save(&DocumentStore::new().current());
        fs::write(&path, &initial).unwrap();
        let expected = FileIdentity::from_bytes(&initial);
        let first_bytes = document_bytes("first");
        let second_bytes = document_bytes("second");
        let (compared_tx, compared_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let first_path = path.clone();
        let first_payload = first_bytes.clone();
        let first_compared_tx = compared_tx.clone();
        let first = std::thread::spawn(move || {
            publish_bytes(
                &first_path,
                &first_payload,
                Publish::IfUnchanged(expected),
                || {
                    first_compared_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                sync_parent_directory,
            )
        });
        compared_rx.recv().unwrap();

        let second_path = path.clone();
        let second_compared_tx = compared_tx;
        let second = std::thread::spawn(move || {
            publish_bytes(
                &second_path,
                &second_bytes,
                Publish::IfUnchanged(expected),
                || second_compared_tx.send(()).unwrap(),
                sync_parent_directory,
            )
        });
        let second_passed_comparison = compared_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        release_tx.send(()).unwrap();

        assert!(first.join().unwrap().is_ok());
        assert!(matches!(
            second.join().unwrap(),
            Err(FilePersistenceError::ExternalConflict)
        ));
        assert!(
            !second_passed_comparison,
            "a second stale writer passed identity comparison before the first writer replaced the file"
        );
        assert_eq!(fs::read(path).unwrap(), first_bytes);
    }

    #[test]
    fn conditional_save_rejects_external_write_after_identity_comparison() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("external-write-race.ketchup");
        let initial = save(&DocumentStore::new().current());
        fs::write(&path, &initial).unwrap();
        let expected = FileIdentity::from_bytes(&initial);

        let mut replacement = DocumentStore::new();
        replacement
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "replacement".into(),
                },
            ]))
            .unwrap();
        let replacement = save(&replacement.current());

        let mut external = DocumentStore::new();
        external
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "external".into(),
                },
            ]))
            .unwrap();
        let external = save(&external.current());
        let external_write = external.clone();

        assert!(matches!(
            publish_bytes(
                &path,
                &replacement,
                Publish::IfUnchanged(expected),
                || fs::write(&path, external_write).unwrap(),
                sync_parent_directory,
            ),
            Err(FilePersistenceError::ExternalConflict)
        ));
        assert_eq!(fs::read(&path).unwrap(), external);
        assert_eq!(fs::read(recovery_path(&path)).unwrap(), initial);
    }

    #[test]
    fn concurrent_work_recovery_and_save_share_identity_lock() {
        use std::sync::mpsc;
        use std::time::Duration;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("concurrent-recovery.ketchup");
        let container_data = ContainerData::default();
        let initial = DocumentStore::new();
        let initial_bytes = save_document_store(&initial, &container_data).unwrap();
        fs::write(&path, &initial_bytes).unwrap();
        let base_identity = FileIdentity::from_bytes(&initial_bytes);

        let mut dirty = DocumentStore::new();
        dirty
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "dirty".into(),
                },
            ]))
            .unwrap();
        let mut replacement = DocumentStore::new();
        replacement
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "replacement".into(),
                },
            ]))
            .unwrap();
        let replacement_bytes = save_document_store(&replacement, &container_data).unwrap();
        let (compared_tx, compared_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let recovery_path = path.clone();
        let recovery_compared_tx = compared_tx.clone();
        let recovery = std::thread::spawn(move || {
            work_recovery::write_work_recovery(
                &recovery_path,
                &dirty,
                &container_data,
                base_identity,
                None,
                || {
                    recovery_compared_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
            )
        });
        compared_rx.recv().unwrap();

        let save_path = path.clone();
        let save_compared_tx = compared_tx;
        let save = std::thread::spawn(move || {
            publish_bytes(
                &save_path,
                &replacement_bytes,
                Publish::IfUnchanged(base_identity),
                || save_compared_tx.send(()).unwrap(),
                sync_parent_directory,
            )
        });
        let save_passed_comparison = compared_rx.recv_timeout(Duration::from_millis(250)).is_ok();
        release_tx.send(()).unwrap();

        assert!(recovery.join().unwrap().is_ok());
        assert!(save.join().unwrap().is_ok());
        assert!(
            !save_passed_comparison,
            "a save passed identity comparison while work recovery was still being published"
        );
    }
}
