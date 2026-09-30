use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DocumentStore, EvaluationIdentity,
    EvaluatorNode, FeatureId, FeatureKind, FeatureParameterFreshnessAudit, NodeId,
    ProposalPrincipal, Revision, RevisionOrigin, Snapshot,
};
use crate::graph::SlotResolution;

mod legacy;
pub(crate) mod snapshot_codec;

pub use legacy::LegacyError;

const MAGIC: &[u8; 10] = b"KETCHUPDOC";
const CONTAINER_MAGIC: &[u8; 10] = b"KETCHUPCTR";
const CONTAINER_SCHEMA: u16 = 1;
const HISTORY_MAGIC: &[u8; 10] = b"KETCHUPHST";
const HISTORY_SCHEMA: u16 = 3;
const WORK_RECOVERY_MAGIC: &[u8; 10] = b"KETCHUPWRK";
const WORK_RECOVERY_SCHEMA: u16 = 1;
const MAX_HISTORY_REVISIONS: u32 = 4_096;
pub const CURRENT_SCHEMA: u16 = snapshot_codec::SNAPSHOT_FORMAT;

const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_COLLECTION_ITEMS: u32 = 500_000;
pub const MAX_NATIVE_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_CONTAINER_BYTES: usize = MAX_NATIVE_DOCUMENT_BYTES;
const MAX_CONTAINER_ENTRIES: u32 = 4_096;
const MAX_CONTAINER_PATH_BYTES: usize = 1_024;
const MAX_SIDECAR_BYTES: usize = 32 * 1024 * 1024;

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
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
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
    blobs: BTreeMap<String, Vec<u8>>,
    imported_source_blobs: Arc<BTreeSet<String>>,
    extensions: BTreeMap<(String, String), ExtensionEntry>,
}

impl ContainerData {
    pub fn insert_blob(&mut self, bytes: Vec<u8>) -> Result<String, PersistenceError> {
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
        let hash = crate::graph::sha256_hex(&bytes);
        self.blobs.entry(hash.clone()).or_insert(bytes);
        Ok(hash)
    }

    pub fn insert_import_blob(&mut self, bytes: Vec<u8>) -> Result<String, PersistenceError> {
        let hash = self.insert_blob(bytes)?;
        Arc::make_mut(&mut self.imported_source_blobs).insert(hash.clone());
        Ok(hash)
    }

    pub fn insert_extension(&mut self, entry: ExtensionEntry) -> Result<(), PersistenceError> {
        let key = (entry.namespace.clone(), entry.path.clone());
        if self.extensions.insert(key, entry).is_some() {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
        Ok(())
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
    pub recovered_from_backup: bool,
}

pub struct ReviewCandidate {
    snapshot: Snapshot,
    audit: Box<LoadAudit>,
    container_data: Box<ContainerData>,
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
                "candidate requires an unknown extension",
            ));
        }
        if self.audit.migration_losses.is_empty() {
            return Err(PersistenceError::MigrationNotConfirmable(
                "candidate has no reported semantic loss",
            ));
        }
        if self
            .audit
            .override_health
            .iter()
            .any(|entry| entry.audited != SlotResolution::Resolved || entry.audited != entry.stored)
        {
            return Err(PersistenceError::MigrationNotConfirmable(
                "candidate has unresolved override identity",
            ));
        }

        let mut migrated_nodes = BTreeSet::new();
        let mut commands = Vec::new();
        for loss in &self.audit.migration_losses {
            if loss.field != "dimension.source_token" || !migrated_nodes.insert(loss.node_id) {
                return Err(PersistenceError::MigrationNotConfirmable(
                    "candidate contains an unsupported migration loss",
                ));
            }
            let dimension = self
                .snapshot
                .evaluator_node(loss.node_id)
                .and_then(EvaluatorNode::dimension)
                .cloned()
                .ok_or(PersistenceError::MigrationNotConfirmable(
                    "migration target is not a parameter dimension",
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

fn append_revision_history_record(
    bytes: &mut Vec<u8>,
    revision: &Revision,
) -> Result<(), PersistenceError> {
    let snapshot = save(revision.snapshot());
    push_u64(bytes, revision.id());
    push_string(bytes, revision.batch_digest());
    write_revision_origin(bytes, revision.origin());
    if let Some(checkpoint) = revision.checkpoint() {
        push_u8(bytes, 1);
        push_string(bytes, checkpoint);
    } else {
        push_u8(bytes, 0);
    }
    if let Some(source) = revision.rule_program() {
        push_u8(bytes, 1);
        let encoded =
            serde_json::to_vec(source).map_err(|_| PersistenceError::InvalidRevisionHistory)?;
        if encoded.len() > crate::document::MAX_RULE_PROGRAM_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
        push_u32(bytes, encoded.len() as u32);
        bytes.extend_from_slice(&encoded);
    } else {
        push_u8(bytes, 0);
    }
    push_u64(bytes, snapshot.len() as u64);
    bytes.extend_from_slice(&crate::graph::sha256_bytes(&snapshot));
    bytes.extend_from_slice(&snapshot);
    if bytes.len() > MAX_SIDECAR_BYTES {
        return Err(PersistenceError::ResourceLimit);
    }
    Ok(())
}

fn encode_revision_history(document: &DocumentStore) -> Result<Vec<u8>, PersistenceError> {
    let count =
        u32::try_from(document.revision_count()).map_err(|_| PersistenceError::ResourceLimit)?;
    if count == 0 || count > MAX_HISTORY_REVISIONS {
        return Err(PersistenceError::ResourceLimit);
    }
    let cursor =
        u32::try_from(document.history_cursor()).map_err(|_| PersistenceError::ResourceLimit)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(HISTORY_MAGIC);
    push_u16(&mut bytes, HISTORY_SCHEMA);
    push_u32(&mut bytes, count);
    push_u32(&mut bytes, cursor);
    push_u64(&mut bytes, document.next_revision_id());
    for revision in document.revision_history() {
        append_revision_history_record(&mut bytes, revision)?;
    }
    Ok(bytes)
}

fn encode_current_revision_history(document: &DocumentStore) -> Result<Vec<u8>, PersistenceError> {
    let revision = document
        .revision_history()
        .nth(document.history_cursor())
        .ok_or(PersistenceError::InvalidRevisionHistory)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(HISTORY_MAGIC);
    push_u16(&mut bytes, HISTORY_SCHEMA);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, 0);
    push_u64(&mut bytes, document.next_revision_id());
    append_revision_history_record(&mut bytes, revision)?;
    Ok(bytes)
}

pub fn save_container(
    snapshot: &Snapshot,
    container_data: &ContainerData,
) -> Result<Vec<u8>, PersistenceError> {
    save_container_entries(
        snapshot,
        container_data,
        imported_source_blob_hashes(snapshot),
        None,
    )
}

pub fn save_document_store(
    document: &DocumentStore,
    container_data: &ContainerData,
) -> Result<Vec<u8>, PersistenceError> {
    let snapshot = document.current();
    let mut imported_sources = BTreeSet::new();
    for revision in document.revision_history() {
        imported_sources.extend(imported_source_blob_hashes(revision.snapshot()));
    }
    save_container_entries(
        &snapshot,
        container_data,
        imported_sources,
        Some(encode_revision_history(document)?),
    )
}

pub fn save_document_store_current_snapshot(
    document: &DocumentStore,
    container_data: &ContainerData,
) -> Result<Vec<u8>, PersistenceError> {
    let snapshot = document.current();
    save_container_entries(
        &snapshot,
        container_data,
        imported_source_blob_hashes(&snapshot),
        Some(encode_current_revision_history(document)?),
    )
}

fn save_container_entries(
    snapshot: &Snapshot,
    container_data: &ContainerData,
    imported_sources: BTreeSet<String>,
    revision_history: Option<Vec<u8>>,
) -> Result<Vec<u8>, PersistenceError> {
    let mut entries = BTreeMap::<String, (bool, Vec<u8>)>::new();
    entries.insert("document.bin".to_owned(), (true, save(snapshot)));
    if let Some(revision_history) = revision_history {
        entries.insert("history.bin".to_owned(), (true, revision_history));
    }
    if imported_sources
        .iter()
        .any(|hash| !container_data.blobs.contains_key(hash))
    {
        return Err(PersistenceError::InvalidBlobHash);
    }
    for (hash, bytes) in &container_data.blobs {
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
    if entries.len() > MAX_CONTAINER_ENTRIES as usize {
        return Err(PersistenceError::ResourceLimit);
    }

    let mut encoded = Vec::new();
    encoded.extend_from_slice(CONTAINER_MAGIC);
    push_u16(&mut encoded, CONTAINER_SCHEMA);
    push_u32(&mut encoded, entries.len() as u32);
    for (path, (required, bytes)) in entries {
        validate_container_path(&path)?;
        if bytes.len() > MAX_SIDECAR_BYTES && path != "document.bin" {
            return Err(PersistenceError::ResourceLimit);
        }
        push_string(&mut encoded, &path);
        push_u8(&mut encoded, u8::from(required));
        push_u64(&mut encoded, bytes.len() as u64);
        encoded.extend_from_slice(&crate::graph::sha256_bytes(&bytes));
        encoded.extend_from_slice(&bytes);
        if encoded.len() > MAX_CONTAINER_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
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
            format!(
                "cleanup target changed concurrently; preserved claimed entry at {}: {error}",
                claimed_path.display()
            ),
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
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum_bytes {
        return Ok(None);
    }
    Ok(Some(FileIdentity::from_bytes(&bytes)))
}

pub fn read_native_document_file(path: impl AsRef<Path>) -> Result<Vec<u8>, FilePersistenceError> {
    let path = path.as_ref();
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_NATIVE_DOCUMENT_BYTES as u64 {
        return Err(FilePersistenceError::Format(
            PersistenceError::ResourceLimit,
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_NATIVE_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_NATIVE_DOCUMENT_BYTES {
        return Err(FilePersistenceError::Format(
            PersistenceError::ResourceLimit,
        ));
    }
    Ok(bytes)
}

pub fn read_native_document_identity(
    path: impl AsRef<Path>,
) -> Result<FileIdentity, FilePersistenceError> {
    read_native_document_file(path).map(|bytes| FileIdentity::from_bytes(&bytes))
}

pub fn save_atomic(
    path: impl AsRef<Path>,
    snapshot: &Snapshot,
) -> Result<(), FilePersistenceError> {
    save_atomic_with_container(path, snapshot, &ContainerData::default())
}

pub fn save_atomic_with_container(
    path: impl AsRef<Path>,
    snapshot: &Snapshot,
    container_data: &ContainerData,
) -> Result<(), FilePersistenceError> {
    let bytes = save_container(snapshot, container_data).map_err(FilePersistenceError::Format)?;
    save_atomic_bytes(path.as_ref(), &bytes, None)
}

pub fn save_atomic_document_store_with_container(
    path: impl AsRef<Path>,
    document: &DocumentStore,
    container_data: &ContainerData,
) -> Result<(), FilePersistenceError> {
    let bytes =
        save_document_store(document, container_data).map_err(FilePersistenceError::Format)?;
    save_atomic_bytes(path.as_ref(), &bytes, None)
}

pub fn save_atomic_document_store_with_container_if_unchanged(
    path: impl AsRef<Path>,
    document: &DocumentStore,
    container_data: &ContainerData,
    expected: FileIdentity,
) -> Result<FileIdentity, FilePersistenceError> {
    let bytes =
        save_document_store(document, container_data).map_err(FilePersistenceError::Format)?;
    save_atomic_bytes(path.as_ref(), &bytes, Some(expected))?;
    Ok(FileIdentity::from_bytes(&bytes))
}

pub fn save_atomic_document_store_with_container_if_absent(
    path: impl AsRef<Path>,
    document: &DocumentStore,
    container_data: &ContainerData,
) -> Result<FileIdentity, FilePersistenceError> {
    let bytes =
        save_document_store(document, container_data).map_err(FilePersistenceError::Format)?;
    save_atomic_bytes_if_absent(path.as_ref(), &bytes)?;
    Ok(FileIdentity::from_bytes(&bytes))
}

pub fn save_atomic_document_store_current_snapshot_with_container(
    path: impl AsRef<Path>,
    document: &DocumentStore,
    container_data: &ContainerData,
) -> Result<(), FilePersistenceError> {
    let bytes = save_document_store_current_snapshot(document, container_data)
        .map_err(FilePersistenceError::Format)?;
    save_atomic_bytes(path.as_ref(), &bytes, None)
}

pub fn save_atomic_document_store_current_snapshot_with_container_if_unchanged(
    path: impl AsRef<Path>,
    document: &DocumentStore,
    container_data: &ContainerData,
    expected: FileIdentity,
) -> Result<FileIdentity, FilePersistenceError> {
    let bytes = save_document_store_current_snapshot(document, container_data)
        .map_err(FilePersistenceError::Format)?;
    save_atomic_bytes(path.as_ref(), &bytes, Some(expected))?;
    Ok(FileIdentity::from_bytes(&bytes))
}

pub fn save_atomic_document_store_current_snapshot_with_container_if_absent(
    path: impl AsRef<Path>,
    document: &DocumentStore,
    container_data: &ContainerData,
) -> Result<FileIdentity, FilePersistenceError> {
    let bytes = save_document_store_current_snapshot(document, container_data)
        .map_err(FilePersistenceError::Format)?;
    save_atomic_bytes_if_absent(path.as_ref(), &bytes)?;
    Ok(FileIdentity::from_bytes(&bytes))
}

fn save_atomic_bytes(
    path: &Path,
    bytes: &[u8],
    expected: Option<FileIdentity>,
) -> Result<(), FilePersistenceError> {
    save_atomic_bytes_after_compare(path, bytes, expected, || {})
}

fn save_atomic_bytes_if_absent(path: &Path, bytes: &[u8]) -> Result<(), FilePersistenceError> {
    save_atomic_bytes_if_absent_after_check(path, bytes, || {}, sync_parent_directory)
}

fn save_atomic_bytes_if_absent_after_check(
    path: &Path,
    bytes: &[u8],
    after_check: impl FnOnce(),
    sync_parent: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), FilePersistenceError> {
    load(bytes).map_err(FilePersistenceError::Format)?;
    let lock_path = save_lock_path(path);
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock_file.lock()?;
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(FilePersistenceError::Io(error)),
        Ok(_) => return Err(FilePersistenceError::ExternalConflict),
    }
    after_check();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            FilePersistenceError::ExternalConflict
        } else {
            FilePersistenceError::Io(error.error)
        }
    })?;
    sync_parent(parent)?;
    Ok(())
}

fn save_atomic_bytes_after_compare(
    path: &Path,
    bytes: &[u8],
    expected: Option<FileIdentity>,
    after_compare: impl FnOnce(),
) -> Result<(), FilePersistenceError> {
    load(bytes).map_err(FilePersistenceError::Format)?;
    let lock_path = save_lock_path(path);
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock_file.lock()?;
    match read_native_document_file(path) {
        Ok(previous)
            if expected.is_some_and(|identity| identity != FileIdentity::from_bytes(&previous)) =>
        {
            return Err(FilePersistenceError::ExternalConflict);
        }
        Ok(previous) if load(&previous).is_ok() => {
            write_atomic(&recovery_path(path), &previous)?;
        }
        Err(FilePersistenceError::Format(PersistenceError::ResourceLimit)) => {
            return Err(FilePersistenceError::Format(
                PersistenceError::ResourceLimit,
            ));
        }
        Err(_) if expected.is_some() => return Err(FilePersistenceError::ExternalConflict),
        _ => {}
    }
    write_atomic_after_prepare(
        path,
        bytes,
        || {
            after_compare();
            if expected.is_some_and(|expected| {
                read_native_document_identity(path).map_or(true, |observed| observed != expected)
            }) {
                return Err(FilePersistenceError::ExternalConflict);
            }
            Ok(())
        },
        sync_parent_directory,
    )
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FilePersistenceError> {
    write_atomic_after_prepare(path, bytes, || Ok(()), sync_parent_directory)
}

fn write_atomic_after_prepare(
    path: &Path,
    bytes: &[u8],
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
    temporary
        .persist(path)
        .map_err(|error| FilePersistenceError::Io(error.error))?;
    sync_parent(parent)?;
    Ok(())
}

fn sync_parent_directory(parent: &Path) -> io::Result<()> {
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

pub fn save_work_recovery_document_store_with_container(
    path: &Path,
    document: &DocumentStore,
    container_data: &ContainerData,
    base_identity: FileIdentity,
) -> Result<FileIdentity, FilePersistenceError> {
    save_work_recovery_document_store_with_container_after_compare(
        path,
        document,
        container_data,
        base_identity,
        || {},
    )
}

fn save_work_recovery_document_store_with_container_after_compare(
    path: &Path,
    document: &DocumentStore,
    container_data: &ContainerData,
    base_identity: FileIdentity,
    after_compare: impl FnOnce(),
) -> Result<FileIdentity, FilePersistenceError> {
    let lock_path = save_lock_path(path);
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock_file.lock()?;
    if read_native_document_identity(path)? != base_identity {
        return Err(FilePersistenceError::ExternalConflict);
    }
    after_compare();
    let payload = match save_document_store(document, container_data) {
        Ok(payload) => payload,
        Err(PersistenceError::ResourceLimit) => {
            save_document_store_current_snapshot(document, container_data)
                .map_err(FilePersistenceError::Format)?
        }
        Err(error) => return Err(FilePersistenceError::Format(error)),
    };
    let mut bytes = Vec::with_capacity(WORK_RECOVERY_MAGIC.len() + 2 + 8 + 32 + 8 + payload.len());
    bytes.extend_from_slice(WORK_RECOVERY_MAGIC);
    push_u16(&mut bytes, WORK_RECOVERY_SCHEMA);
    push_u64(&mut bytes, base_identity.byte_len());
    bytes.extend_from_slice(&base_identity.sha256());
    push_u64(&mut bytes, payload.len() as u64);
    bytes.extend_from_slice(&payload);
    write_atomic(&work_recovery_path(path), &bytes)?;
    Ok(FileIdentity::from_bytes(&bytes))
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
    let lock_path = save_lock_path(path);
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock_file.lock()?;
    let recovery_path = work_recovery_path(path);
    let metadata = match fs::symlink_metadata(&recovery_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(FilePersistenceError::Io(error)),
    };
    if !metadata.file_type().is_file() {
        return Err(FilePersistenceError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "work-recovery path is not a regular file",
        )));
    }
    let Some(expected) = expected else {
        return Ok(false);
    };
    remove_regular_file_if_unchanged_after_check(
        &recovery_path,
        expected,
        MAX_NATIVE_DOCUMENT_BYTES as u64 + 60,
        after_compare,
    )
    .map_err(FilePersistenceError::Io)
}

fn recovery_path(path: &Path) -> PathBuf {
    let mut recovery = path.as_os_str().to_os_string();
    recovery.push(".recovery");
    PathBuf::from(recovery)
}

fn save_lock_path(path: &Path) -> PathBuf {
    let mut lock = path.as_os_str().to_os_string();
    lock.push(".save-lock");
    PathBuf::from(lock)
}

fn try_load_work_recovery(
    path: &Path,
    base_identity: FileIdentity,
) -> Result<Option<LoadedFile>, FilePersistenceError> {
    let source_path = work_recovery_path(path);
    let mut file = match fs::File::open(&source_path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(FilePersistenceError::Io(error)),
    };
    let wrapper_limit = MAX_NATIVE_DOCUMENT_BYTES as u64 + 60;
    if file.metadata()?.len() > wrapper_limit {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(wrapper_limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > wrapper_limit {
        return Ok(None);
    }
    let mut reader = Reader::new(&bytes);
    if reader.take(WORK_RECOVERY_MAGIC.len()).ok() != Some(WORK_RECOVERY_MAGIC)
        || reader.u16().ok() != Some(WORK_RECOVERY_SCHEMA)
    {
        return Ok(None);
    }
    let Some(byte_len) = reader.u64().ok() else {
        return Ok(None);
    };
    let Some(sha256) = reader.take(32).ok().and_then(|value| value.try_into().ok()) else {
        return Ok(None);
    };
    if (FileIdentity { byte_len, sha256 }) != base_identity {
        return Ok(None);
    }
    let Some(payload_len) = reader
        .u64()
        .ok()
        .and_then(|value| usize::try_from(value).ok())
    else {
        return Ok(None);
    };
    if payload_len > MAX_NATIVE_DOCUMENT_BYTES {
        return Ok(None);
    }
    let Some(payload) = reader.take(payload_len).ok() else {
        return Ok(None);
    };
    if !reader.is_finished() {
        return Ok(None);
    }
    let mut outcome = match load(payload) {
        Ok(outcome) => outcome,
        Err(_) => return Ok(None),
    };
    match &mut outcome {
        LoadOutcome::Editable { audit, .. } => audit.recovered_from_backup = true,
        LoadOutcome::ReviewOnly(candidate) => candidate.audit.recovered_from_backup = true,
    }
    Ok(Some(LoadedFile {
        outcome,
        source_path,
        source_bytes: payload.to_vec(),
        work_recovery_identity: Some(FileIdentity::from_bytes(&bytes)),
    }))
}

pub struct LoadedFile {
    outcome: LoadOutcome,
    source_path: PathBuf,
    source_bytes: Vec<u8>,
    work_recovery_identity: Option<FileIdentity>,
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
            Ok(outcome) => {
                let identity = FileIdentity::from_bytes(&bytes);
                match try_load_work_recovery(path, identity) {
                    Ok(Some(recovered)) => Ok(recovered),
                    Ok(None) | Err(_) => Ok(LoadedFile {
                        outcome,
                        source_path: path.to_owned(),
                        source_bytes: bytes,
                        work_recovery_identity: None,
                    }),
                }
            }
            Err(primary_error @ PersistenceError::LegacyFeatureRequiresMigration { .. }) => {
                Err(FilePersistenceError::Format(primary_error))
            }
            Err(primary_error) => {
                try_load_recovery(path)?.ok_or(FilePersistenceError::Format(primary_error))
            }
        },
        Err(error @ FilePersistenceError::Format(PersistenceError::ResourceLimit)) => Err(error),
        Err(FilePersistenceError::Io(primary_error)) => {
            try_load_recovery(path)?.ok_or(FilePersistenceError::Io(primary_error))
        }
        Err(FilePersistenceError::Format(error)) => Err(FilePersistenceError::Format(error)),
        Err(FilePersistenceError::ExternalConflict) => Err(FilePersistenceError::ExternalConflict),
    }
}

fn try_load_recovery(path: &Path) -> Result<Option<LoadedFile>, FilePersistenceError> {
    let source_path = recovery_path(path);
    let bytes = match read_native_document_file(&source_path) {
        Ok(bytes) => bytes,
        Err(error @ FilePersistenceError::Format(PersistenceError::ResourceLimit)) => {
            return Err(error);
        }
        Err(FilePersistenceError::Io(_)) => return Ok(None),
        Err(FilePersistenceError::Format(error)) => {
            return Err(FilePersistenceError::Format(error));
        }
        Err(FilePersistenceError::ExternalConflict) => {
            return Err(FilePersistenceError::ExternalConflict);
        }
    };
    let mut outcome = match load(&bytes) {
        Ok(outcome) => outcome,
        Err(_) => return Ok(None),
    };
    match &mut outcome {
        LoadOutcome::Editable { audit, .. } => audit.recovered_from_backup = true,
        LoadOutcome::ReviewOnly(candidate) => candidate.audit.recovered_from_backup = true,
    }
    Ok(Some(LoadedFile {
        outcome,
        source_path,
        source_bytes: bytes,
        work_recovery_identity: None,
    }))
}

pub fn load(bytes: &[u8]) -> Result<LoadOutcome, PersistenceError> {
    if bytes.starts_with(CONTAINER_MAGIC) {
        return load_container(bytes);
    }
    load_document(bytes, ContainerData::default(), &mut BTreeMap::new())
}

fn load_container(bytes: &[u8]) -> Result<LoadOutcome, PersistenceError> {
    if bytes.len() > MAX_CONTAINER_BYTES {
        return Err(PersistenceError::ResourceLimit);
    }
    let mut reader = Reader::new(bytes);
    if reader.take(CONTAINER_MAGIC.len())? != CONTAINER_MAGIC {
        return Err(PersistenceError::InvalidContainerMagic);
    }
    let schema = reader.u16()?;
    if schema != CONTAINER_SCHEMA {
        return Err(PersistenceError::UnsupportedContainerSchema(schema));
    }
    let entry_count = reader.count_with_limit(MAX_CONTAINER_ENTRIES)?;
    let mut entries = BTreeMap::<String, (bool, Vec<u8>)>::new();
    for _ in 0..entry_count {
        let path = reader.string()?;
        validate_container_path(&path)?;
        let required = reader.boolean()?;
        let length =
            usize::try_from(reader.u64()?).map_err(|_| PersistenceError::LengthOverflow)?;
        if length > MAX_SIDECAR_BYTES && path != "document.bin" {
            return Err(PersistenceError::ResourceLimit);
        }
        let checksum: [u8; 32] = reader
            .take(32)?
            .try_into()
            .map_err(|_| PersistenceError::Truncated)?;
        let content = reader.take(length)?.to_vec();
        if crate::graph::sha256_bytes(&content) != checksum {
            return Err(PersistenceError::ContainerChecksumMismatch(path));
        }
        if entries.insert(path, (required, content)).is_some() {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
    }
    if !reader.is_finished() {
        return Err(PersistenceError::TrailingBytes);
    }

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
    let mut container_data = ContainerData::default();
    for (path, (required, content)) in entries {
        if let Some(hash) = path.strip_prefix("blobs/") {
            if required || hash.len() != 64 || crate::graph::sha256_hex(&content) != hash {
                return Err(PersistenceError::InvalidBlobHash);
            }
            if container_data
                .blobs
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
        blobs: container_data.blobs.clone(),
        imported_source_blobs: Arc::new(BTreeSet::new()),
        extensions: BTreeMap::new(),
    };
    let mut outcome = load_document(&document, container_data, &mut BTreeMap::new())?;
    if let Some(revision_history) = revision_history {
        let restored =
            decode_revision_history(&revision_history, &outcome, &history_container_data)?;
        if let LoadOutcome::Editable { document, .. } = &mut outcome {
            *document = restored;
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
    expected_current: &LoadOutcome,
    container_data: &ContainerData,
) -> Result<DocumentStore, PersistenceError> {
    if bytes.len() > MAX_SIDECAR_BYTES {
        return Err(PersistenceError::ResourceLimit);
    }
    let mut reader = Reader::new(bytes);
    if reader.take(HISTORY_MAGIC.len())? != HISTORY_MAGIC {
        return Err(PersistenceError::InvalidHistoryMagic);
    }
    let schema = reader.u16()?;
    if schema != 1 && schema != 2 && schema != HISTORY_SCHEMA {
        return Err(PersistenceError::UnsupportedHistorySchema(schema));
    }
    let count = reader.count_with_limit(MAX_HISTORY_REVISIONS)? as usize;
    let cursor = reader.u32()? as usize;
    let next_revision_id = reader.u64()?;
    if count == 0 || cursor >= count {
        return Err(PersistenceError::InvalidRevisionHistory);
    }

    let mut revisions = Vec::with_capacity(count);
    let mut previous_revision_id = None;
    let mut document_id = None;
    let mut checkpoint_names = BTreeSet::new();
    let mut migrated_digests = BTreeMap::new();
    let mut current_source_digest = None;
    for index in 0..count {
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
                    let source: crate::document::RuleProgramSource =
                        serde_json::from_slice(reader.take(length)?)
                            .map_err(|_| PersistenceError::InvalidRevisionHistory)?;
                    if source.source.is_empty()
                        || source.overrides.values().any(|value| !value.is_finite())
                    {
                        return Err(PersistenceError::InvalidRevisionHistory);
                    }
                    Some(source)
                }
                _ => return Err(PersistenceError::InvalidRevisionHistory),
            }
        } else {
            None
        };
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
        let length =
            usize::try_from(reader.u64()?).map_err(|_| PersistenceError::LengthOverflow)?;
        if length > MAX_FILE_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
        let checksum: [u8; 32] = reader
            .take(32)?
            .try_into()
            .map_err(|_| PersistenceError::Truncated)?;
        let encoded_snapshot = reader.take(length)?;
        if crate::graph::sha256_bytes(encoded_snapshot) != checksum {
            return Err(PersistenceError::HistoryChecksumMismatch);
        }
        let loaded = load_document(
            encoded_snapshot,
            container_data.clone(),
            &mut migrated_digests,
        )?;
        if !loaded.is_editable() {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        let source_schema = loaded.source_schema();
        if index == cursor {
            current_source_digest = Some(loaded.audit().source_canonical_digest.clone());
        }
        let snapshot = loaded
            .into_editable()
            .map_err(|_| PersistenceError::InvalidRevisionHistory)?
            .current();
        if snapshot.revision_id() != revision_id
            || (source_schema == CURRENT_SCHEMA && save(&snapshot) != encoded_snapshot)
        {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        if document_id.is_some_and(|id| id != snapshot.document_id()) {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        document_id = Some(snapshot.document_id());
        previous_revision_id = Some(revision_id);
        revisions.push((snapshot, batch_digest, origin, checkpoint, rule_program));
    }
    if !reader.is_finished()
        || previous_revision_id.is_none_or(|revision_id| next_revision_id <= revision_id)
    {
        return Err(PersistenceError::InvalidRevisionHistory);
    }
    // The revision a history names as current is the saved document. Both were written by
    // one version, so they agree on the digest that version computed.
    let current = &revisions[cursor].0;
    let expected_snapshot = expected_current.snapshot();
    if current.document_id() != expected_snapshot.document_id()
        || current.revision_id() != expected_snapshot.revision_id()
        || current_source_digest.as_deref()
            != Some(expected_current.audit().source_canonical_digest.as_str())
        || (expected_current.source_schema() == CURRENT_SCHEMA
            && save(current) != save(&expected_snapshot))
    {
        return Err(PersistenceError::InvalidRevisionHistory);
    }
    DocumentStore::from_revision_history(revisions, cursor, next_revision_id)
        .map_err(PersistenceError::InvalidCanonicalData)
}

fn load_document(
    bytes: &[u8],
    mut container_data: ContainerData,
    migrated_digests: &mut BTreeMap<String, String>,
) -> Result<LoadOutcome, PersistenceError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(PersistenceError::ResourceLimit);
    }
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
            &decoded.point_profiles,
            migrated_digests,
            |product| legacy::complete_old_records(product, &decoded.point_profiles),
        )
        .map_err(|error| PersistenceError::InvalidPayload(error.to_string()))?;
        (
            decoded.revision_id,
            migrated.product,
            decoded.migration_losses,
            Some(migrated.source_digest),
        )
    };
    if schema < CURRENT_SCHEMA {
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
        recovered_from_backup: false,
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
    // A stored face or edge reference must still name the body its producer
    // builds. Evaluator digests may differ: older files were evaluated by other
    // code paths, and the next evaluation refreshes the evidence.
    for reference in loaded_snapshot.exact_reference_evidence() {
        let names_current_body = crate::exact_brep_graph::ExactBRepGraph::from_snapshot(
            &loaded_snapshot,
            reference.definition_id,
            reference.producer_feature_id,
        )
        .is_ok_and(|graph| graph.names_durable_reference(reference));
        if !names_current_body {
            return Err(PersistenceError::InvalidExactReference);
        }
    }
    if review_required || container_data.requires_unknown_extension() {
        Ok(LoadOutcome::ReviewOnly(ReviewCandidate {
            snapshot: document.current(),
            audit: Box::new(audit),
            container_data: Box::new(container_data),
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
}

impl fmt::Display for FilePersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Format(error) => error.fmt(formatter),
            Self::ExternalConflict => {
                formatter.write_str("the document changed outside this session")
            }
        }
    }
}

impl std::error::Error for FilePersistenceError {}
impl From<io::Error> for FilePersistenceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PersistenceError {
    /// A pre-serde document failed the frozen legacy reader.
    Legacy(LegacyError),
    MigrationNotConfirmable(&'static str),
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
    DuplicateContainerEntry,
    InvalidContainerPath(String),
    UnsupportedContainerEntry(String),
    ContainerChecksumMismatch(String),
    InvalidBlobHash,
    InvalidUtf8,
    LengthOverflow,
    TrailingBytes,
    InvalidBoolean(u8),
    InvalidExactReference,
    ChecksumMismatch,
    InvalidPayload(String),
    ResourceLimit,
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
            Self::InvalidUtf8 => formatter.write_str("document string is not UTF-8"),
            Self::LengthOverflow => formatter.write_str("document length exceeds this platform"),
            Self::TrailingBytes => formatter.write_str("document has trailing bytes"),
            Self::InvalidBoolean(value) => write!(formatter, "document boolean {value} is invalid"),
            Self::InvalidExactReference => {
                formatter.write_str("exact reference evidence is invalid")
            }
            Self::ChecksumMismatch => formatter.write_str("document checksum does not match"),
            Self::InvalidPayload(reason) => {
                write!(formatter, "document payload is invalid: {reason}")
            }
            Self::ResourceLimit => formatter.write_str("document exceeds a resource limit"),
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

impl std::error::Error for PersistenceError {}
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
    string_bytes: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            cursor: 0,
            collection_items: 0,
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
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| PersistenceError::Truncated)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, PersistenceError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| PersistenceError::Truncated)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, PersistenceError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| PersistenceError::Truncated)?,
        ))
    }
    fn count_with_limit(&mut self, limit: u32) -> Result<u32, PersistenceError> {
        let value = self.u32()?;
        self.collection_items = self
            .collection_items
            .checked_add(u64::from(value))
            .ok_or(PersistenceError::ResourceLimit)?;
        if value > limit || self.collection_items > u64::from(MAX_COLLECTION_ITEMS) {
            Err(PersistenceError::ResourceLimit)
        } else {
            Ok(value)
        }
    }
    fn string(&mut self) -> Result<String, PersistenceError> {
        let length = usize::try_from(self.count_with_limit(MAX_STRING_BYTES as u32)?)
            .map_err(|_| PersistenceError::LengthOverflow)?;
        self.string_bytes = self
            .string_bytes
            .checked_add(length)
            .ok_or(PersistenceError::ResourceLimit)?;
        if self.string_bytes > MAX_STRING_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
        let value =
            std::str::from_utf8(self.take(length)?).map_err(|_| PersistenceError::InvalidUtf8)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(length)
            .map_err(|_| PersistenceError::ResourceLimit)?;
        owned.push_str(value);
        Ok(owned)
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
fn push_string(bytes: &mut Vec<u8>, value: &str) {
    push_u32(bytes, value.len() as u32);
    bytes.extend_from_slice(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DefinitionId;

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
            save_atomic_bytes_if_absent_after_check(
                &path,
                &payload,
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

        let error = save_atomic_bytes_if_absent_after_check(
            &path,
            &payload,
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
            save_atomic_bytes_after_compare(&first_path, &first_payload, Some(expected), || {
                first_compared_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
        });
        compared_rx.recv().unwrap();

        let second_path = path.clone();
        let second_compared_tx = compared_tx;
        let second = std::thread::spawn(move || {
            save_atomic_bytes_after_compare(&second_path, &second_bytes, Some(expected), || {
                second_compared_tx.send(()).unwrap()
            })
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
            save_atomic_bytes_after_compare(&path, &replacement, Some(expected), || {
                fs::write(&path, external_write).unwrap();
            }),
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
            save_work_recovery_document_store_with_container_after_compare(
                &recovery_path,
                &dirty,
                &container_data,
                base_identity,
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
            save_atomic_bytes_after_compare(
                &save_path,
                &replacement_bytes,
                Some(base_identity),
                || save_compared_tx.send(()).unwrap(),
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
