use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::{Snapshot, UnitSystem};
use crate::graph::sha256_hex;
use crate::persistence::{self, PersistenceError};

pub const LOCAL_PDM_RELEASE_SCHEMA_V1: &str = "ketchup.local-pdm.release.v1";
pub const MAX_RELEASE_DEPENDENCIES: usize = 256;
pub const MAX_RELEASE_DEPENDENCY_BYTES: usize = 256 * 1024 * 1024;
const MAX_RELEASE_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_LOGICAL_PATH_BYTES: usize = 512;
const MAX_AUDIT_ACTOR_BYTES: usize = 128;
const MAX_AUDIT_NOTE_BYTES: usize = 2_048;
const MAX_RELEASE_CATALOG_ENTRIES: usize = 4_096;
const MAX_RELEASE_LINEAGE_DEPTH: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseDependencyInput {
    pub logical_path: String,
    pub source_path: PathBuf,
}

impl ReleaseDependencyInput {
    #[must_use]
    pub fn new(logical_path: impl Into<String>, source_path: impl Into<PathBuf>) -> Self {
        Self {
            logical_path: logical_path.into(),
            source_path: source_path.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAudit {
    pub actor: String,
    pub created_unix_ms: u64,
    pub note: String,
}

impl ReleaseAudit {
    #[must_use]
    pub fn new(actor: impl Into<String>, created_unix_ms: u64, note: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            created_unix_ms,
            note: note.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseObjectIdentity {
    pub sha256: String,
    pub byte_len: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasedDocument {
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub units: String,
    pub object: ReleaseObjectIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasedDependency {
    pub logical_path: String,
    pub object: ReleaseObjectIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleasePayload {
    schema: String,
    parent_release_id: Option<String>,
    document: ReleasedDocument,
    dependencies: Vec<ReleasedDependency>,
    audit: ReleaseAudit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema: String,
    pub release_id: String,
    pub parent_release_id: Option<String>,
    pub document: ReleasedDocument,
    pub dependencies: Vec<ReleasedDependency>,
    pub audit: ReleaseAudit,
}

impl ReleaseManifest {
    fn payload(&self) -> ReleasePayload {
        ReleasePayload {
            schema: self.schema.clone(),
            parent_release_id: self.parent_release_id.clone(),
            document: self.document.clone(),
            dependencies: self.dependencies.clone(),
            audit: self.audit.clone(),
        }
    }

    fn computed_release_id(&self) -> Result<String, LocalPdmError> {
        canonical_payload_identity(&self.payload())
    }
}

#[derive(Clone)]
pub struct VerifiedRelease {
    pub manifest: ReleaseManifest,
    pub snapshot: Snapshot,
    pub dependency_objects: BTreeMap<String, PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseCatalogEntry {
    pub release_id: String,
    pub parent_release_id: Option<String>,
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub dependency_count: usize,
    pub audit: ReleaseAudit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyChange {
    pub logical_path: String,
    pub kind: DependencyChangeKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseRelationship {
    Same,
    LeftAncestor,
    RightAncestor,
    Diverged { common_ancestor: String },
    Unrelated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseConflictVerdict {
    AlreadyCurrent,
    FastForward,
    IncomingBehind,
    DivergedConflict,
    UnrelatedConflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseComparison {
    pub left_release_id: String,
    pub right_release_id: String,
    pub relationship: ReleaseRelationship,
    pub conflict_verdict: ReleaseConflictVerdict,
    pub document_changed: bool,
    pub dependency_changes: Vec<DependencyChange>,
}

#[derive(Debug)]
pub enum LocalPdmError {
    Io(io::Error),
    Json(serde_json::Error),
    Persistence(PersistenceError),
    StaleSnapshot,
    InvalidAudit,
    TooManyDependencies,
    TooManyReleases,
    LineageTooDeep,
    ParentDocumentMismatch,
    NoChangesAgainstParent,
    InvalidLogicalPath,
    DuplicateLogicalPath,
    DependencyTooLarge,
    InvalidReleaseId,
    ManifestTooLarge,
    InvalidManifest,
    ManifestIdentityMismatch,
    ReleaseAlreadyExists,
    MissingRelease { release_id: String },
    MissingObject { sha256: String },
    ObjectTampered { sha256: String },
    ObjectConflict { sha256: String },
    DocumentIdentityMismatch,
}

impl fmt::Display for LocalPdmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "local PDM I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "local PDM manifest JSON is invalid: {error}"),
            Self::Persistence(error) => write!(formatter, "released document is invalid: {error}"),
            Self::StaleSnapshot => formatter.write_str("release request is stale"),
            Self::InvalidAudit => formatter.write_str("release audit metadata is invalid"),
            Self::TooManyDependencies => formatter.write_str("release has too many dependencies"),
            Self::TooManyReleases => {
                formatter.write_str("local release catalog exceeds its resource bound")
            }
            Self::LineageTooDeep => {
                formatter.write_str("local release lineage exceeds its resource bound")
            }
            Self::ParentDocumentMismatch => {
                formatter.write_str("parent release belongs to another document")
            }
            Self::NoChangesAgainstParent => {
                formatter.write_str("child release has no document or dependency changes")
            }
            Self::InvalidLogicalPath => formatter.write_str("dependency logical path is invalid"),
            Self::DuplicateLogicalPath => {
                formatter.write_str("dependency logical path is duplicated")
            }
            Self::DependencyTooLarge => {
                formatter.write_str("release dependency exceeds its resource bound")
            }
            Self::InvalidReleaseId => formatter.write_str("release identifier is invalid"),
            Self::ManifestTooLarge => {
                formatter.write_str("release manifest exceeds its resource bound")
            }
            Self::InvalidManifest => {
                formatter.write_str("release manifest violates the canonical contract")
            }
            Self::ManifestIdentityMismatch => {
                formatter.write_str("release manifest identity does not match its content")
            }
            Self::ReleaseAlreadyExists => formatter.write_str("immutable release already exists"),
            Self::MissingRelease { release_id } => {
                write!(formatter, "parent release {release_id} is missing")
            }
            Self::MissingObject { sha256 } => {
                write!(formatter, "release object {sha256} is missing")
            }
            Self::ObjectTampered { sha256 } => {
                write!(formatter, "release object {sha256} was modified")
            }
            Self::ObjectConflict { sha256 } => write!(
                formatter,
                "content-addressed object {sha256} conflicts with existing content"
            ),
            Self::DocumentIdentityMismatch => {
                formatter.write_str("released document identity does not match the manifest")
            }
        }
    }
}

impl std::error::Error for LocalPdmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Persistence(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for LocalPdmError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for LocalPdmError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<PersistenceError> for LocalPdmError {
    fn from(error: PersistenceError) -> Self {
        Self::Persistence(error)
    }
}

pub fn create_release(
    repository: impl AsRef<Path>,
    snapshot: &Snapshot,
    expected_revision: u64,
    expected_canonical_digest: &str,
    dependencies: &[ReleaseDependencyInput],
    audit: ReleaseAudit,
) -> Result<ReleaseManifest, LocalPdmError> {
    create_release_internal(
        repository.as_ref(),
        None,
        snapshot,
        expected_revision,
        expected_canonical_digest,
        dependencies,
        audit,
    )
}

pub fn create_child_release(
    repository: impl AsRef<Path>,
    parent_release_id: &str,
    snapshot: &Snapshot,
    expected_revision: u64,
    expected_canonical_digest: &str,
    dependencies: &[ReleaseDependencyInput],
    audit: ReleaseAudit,
) -> Result<ReleaseManifest, LocalPdmError> {
    let repository = repository.as_ref();
    let parent = open_release(repository, parent_release_id)?.manifest;
    if parent.document.document_id != snapshot.document_id().0 {
        return Err(LocalPdmError::ParentDocumentMismatch);
    }
    create_release_internal(
        repository,
        Some(&parent),
        snapshot,
        expected_revision,
        expected_canonical_digest,
        dependencies,
        audit,
    )
}

fn create_release_internal(
    repository: &Path,
    parent: Option<&ReleaseManifest>,
    snapshot: &Snapshot,
    expected_revision: u64,
    expected_canonical_digest: &str,
    dependencies: &[ReleaseDependencyInput],
    audit: ReleaseAudit,
) -> Result<ReleaseManifest, LocalPdmError> {
    if snapshot.revision_id() != expected_revision
        || snapshot.canonical_digest() != expected_canonical_digest
    {
        return Err(LocalPdmError::StaleSnapshot);
    }
    validate_audit(&audit)?;
    if dependencies.len() > MAX_RELEASE_DEPENDENCIES {
        return Err(LocalPdmError::TooManyDependencies);
    }

    let document_bytes = persistence::save(snapshot);
    let document_object = identity(&document_bytes);
    let mut dependency_objects = Vec::with_capacity(dependencies.len());
    let mut logical_paths = BTreeSet::new();
    for dependency in dependencies {
        validate_logical_path(&dependency.logical_path)?;
        if !logical_paths.insert(dependency.logical_path.clone()) {
            return Err(LocalPdmError::DuplicateLogicalPath);
        }
        let bytes = read_bounded_file(
            &dependency.source_path,
            MAX_RELEASE_DEPENDENCY_BYTES,
            LocalPdmError::DependencyTooLarge,
        )?;
        dependency_objects.push((
            ReleasedDependency {
                logical_path: dependency.logical_path.clone(),
                object: identity(&bytes),
            },
            bytes,
        ));
    }
    dependency_objects.sort_by(|left, right| left.0.logical_path.cmp(&right.0.logical_path));

    let payload = ReleasePayload {
        schema: LOCAL_PDM_RELEASE_SCHEMA_V1.to_owned(),
        parent_release_id: parent.map(|manifest| manifest.release_id.clone()),
        document: ReleasedDocument {
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            units: match snapshot.units() {
                UnitSystem::Millimetres => "millimetres".to_owned(),
            },
            object: document_object.clone(),
        },
        dependencies: dependency_objects
            .iter()
            .map(|(dependency, _)| dependency.clone())
            .collect(),
        audit,
    };
    if parent.is_some_and(|manifest| {
        manifest.document.object == payload.document.object
            && manifest.dependencies == payload.dependencies
    }) {
        return Err(LocalPdmError::NoChangesAgainstParent);
    }
    let release_id = canonical_payload_identity(&payload)?;
    let manifest = ReleaseManifest {
        schema: payload.schema,
        release_id: release_id.clone(),
        parent_release_id: payload.parent_release_id,
        document: payload.document,
        dependencies: payload.dependencies,
        audit: payload.audit,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    if manifest_bytes.len() > MAX_RELEASE_MANIFEST_BYTES {
        return Err(LocalPdmError::ManifestTooLarge);
    }

    write_content_addressed_object(repository, &document_object, &document_bytes)?;
    for (dependency, bytes) in &dependency_objects {
        write_content_addressed_object(repository, &dependency.object, bytes)?;
    }
    write_manifest(repository, &release_id, &manifest_bytes)?;
    Ok(manifest)
}

pub fn open_release(
    repository: impl AsRef<Path>,
    release_id: &str,
) -> Result<VerifiedRelease, LocalPdmError> {
    let repository = repository.as_ref();
    let manifest = read_manifest(repository, release_id)?;

    let document_bytes = read_object(repository, &manifest.document.object)?;
    let loaded = persistence::load(&document_bytes)?;
    let snapshot = loaded.snapshot().clone();
    if snapshot.document_id().0 != manifest.document.document_id
        || snapshot.revision_id() != manifest.document.revision
        || snapshot.canonical_digest() != manifest.document.canonical_digest
        || manifest.document.units != "millimetres"
    {
        return Err(LocalPdmError::DocumentIdentityMismatch);
    }

    let mut dependency_objects = BTreeMap::new();
    for dependency in &manifest.dependencies {
        read_object(repository, &dependency.object)?;
        dependency_objects.insert(
            dependency.logical_path.clone(),
            release_object_path(repository, &dependency.object.sha256)?,
        );
    }
    Ok(VerifiedRelease {
        manifest,
        snapshot,
        dependency_objects,
    })
}

pub fn release_catalog(
    repository: impl AsRef<Path>,
) -> Result<Vec<ReleaseCatalogEntry>, LocalPdmError> {
    let repository = repository.as_ref();
    let releases_path = repository.join("releases");
    let entries = match fs::read_dir(&releases_path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(LocalPdmError::Io(error)),
    };
    let mut release_ids = Vec::new();
    for entry in entries {
        let entry = entry?;
        if release_ids.len() == MAX_RELEASE_CATALOG_ENTRIES {
            return Err(LocalPdmError::TooManyReleases);
        }
        if !entry.file_type()?.is_file() {
            return Err(LocalPdmError::InvalidManifest);
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| LocalPdmError::InvalidManifest)?;
        let release_id = name
            .strip_suffix(".json")
            .ok_or(LocalPdmError::InvalidManifest)?;
        validate_sha256(release_id)?;
        release_ids.push(release_id.to_owned());
    }
    release_ids.sort();
    let mut catalog = Vec::with_capacity(release_ids.len());
    for release_id in release_ids {
        let manifest = read_manifest(repository, &release_id)?;
        catalog.push(ReleaseCatalogEntry {
            release_id: manifest.release_id,
            parent_release_id: manifest.parent_release_id,
            document_id: manifest.document.document_id,
            revision: manifest.document.revision,
            canonical_digest: manifest.document.canonical_digest,
            dependency_count: manifest.dependencies.len(),
            audit: manifest.audit,
        });
    }
    validate_catalog_lineage(&catalog)?;
    catalog.sort_by(|left, right| {
        left.audit
            .created_unix_ms
            .cmp(&right.audit.created_unix_ms)
            .then_with(|| left.release_id.cmp(&right.release_id))
    });
    Ok(catalog)
}

fn validate_catalog_lineage(catalog: &[ReleaseCatalogEntry]) -> Result<(), LocalPdmError> {
    let by_id = catalog
        .iter()
        .map(|entry| (entry.release_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    for entry in catalog {
        let mut seen = BTreeSet::new();
        let mut current = entry;
        loop {
            if seen.len() == MAX_RELEASE_LINEAGE_DEPTH {
                return Err(LocalPdmError::LineageTooDeep);
            }
            if !seen.insert(current.release_id.as_str()) {
                return Err(LocalPdmError::InvalidManifest);
            }
            let Some(parent_release_id) = &current.parent_release_id else {
                break;
            };
            let parent = by_id.get(parent_release_id.as_str()).ok_or_else(|| {
                LocalPdmError::MissingRelease {
                    release_id: parent_release_id.clone(),
                }
            })?;
            if parent.document_id != entry.document_id {
                return Err(LocalPdmError::ParentDocumentMismatch);
            }
            current = parent;
        }
    }
    Ok(())
}

pub fn compare_releases(
    repository: impl AsRef<Path>,
    left_release_id: &str,
    right_release_id: &str,
) -> Result<ReleaseComparison, LocalPdmError> {
    let repository = repository.as_ref();
    let left = read_manifest(repository, left_release_id)?;
    let right = read_manifest(repository, right_release_id)?;
    let relationship = if left.release_id == right.release_id {
        release_ancestry(repository, &left)?;
        ReleaseRelationship::Same
    } else if left.document.document_id != right.document.document_id {
        release_ancestry(repository, &left)?;
        release_ancestry(repository, &right)?;
        ReleaseRelationship::Unrelated
    } else {
        release_relationship(repository, &left, &right)?
    };
    let conflict_verdict = match relationship {
        ReleaseRelationship::Same => ReleaseConflictVerdict::AlreadyCurrent,
        ReleaseRelationship::LeftAncestor => ReleaseConflictVerdict::FastForward,
        ReleaseRelationship::RightAncestor => ReleaseConflictVerdict::IncomingBehind,
        ReleaseRelationship::Diverged { .. } => ReleaseConflictVerdict::DivergedConflict,
        ReleaseRelationship::Unrelated => ReleaseConflictVerdict::UnrelatedConflict,
    };

    let left_dependencies = left
        .dependencies
        .iter()
        .map(|dependency| (dependency.logical_path.as_str(), &dependency.object))
        .collect::<BTreeMap<_, _>>();
    let right_dependencies = right
        .dependencies
        .iter()
        .map(|dependency| (dependency.logical_path.as_str(), &dependency.object))
        .collect::<BTreeMap<_, _>>();
    let paths = left_dependencies
        .keys()
        .chain(right_dependencies.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut dependency_changes = Vec::new();
    for path in paths {
        let kind = match (left_dependencies.get(path), right_dependencies.get(path)) {
            (None, Some(_)) => Some(DependencyChangeKind::Added),
            (Some(_), None) => Some(DependencyChangeKind::Removed),
            (Some(left), Some(right)) if left != right => Some(DependencyChangeKind::Modified),
            _ => None,
        };
        if let Some(kind) = kind {
            dependency_changes.push(DependencyChange {
                logical_path: path.to_owned(),
                kind,
            });
        }
    }

    Ok(ReleaseComparison {
        left_release_id: left.release_id,
        right_release_id: right.release_id,
        relationship,
        conflict_verdict,
        document_changed: left.document.object != right.document.object
            || left.document.canonical_digest != right.document.canonical_digest,
        dependency_changes,
    })
}

fn release_relationship(
    repository: &Path,
    left: &ReleaseManifest,
    right: &ReleaseManifest,
) -> Result<ReleaseRelationship, LocalPdmError> {
    let left_ancestry = release_ancestry(repository, left)?;
    let right_ancestry = release_ancestry(repository, right)?;
    if right_ancestry.contains(&left.release_id) {
        return Ok(ReleaseRelationship::LeftAncestor);
    }
    if left_ancestry.contains(&right.release_id) {
        return Ok(ReleaseRelationship::RightAncestor);
    }
    let right_ids = right_ancestry.into_iter().collect::<BTreeSet<_>>();
    if let Some(common_ancestor) = left_ancestry
        .into_iter()
        .find(|release_id| right_ids.contains(release_id))
    {
        return Ok(ReleaseRelationship::Diverged { common_ancestor });
    }
    Ok(ReleaseRelationship::Unrelated)
}

fn release_ancestry(
    repository: &Path,
    release: &ReleaseManifest,
) -> Result<Vec<String>, LocalPdmError> {
    let document_id = release.document.document_id;
    let mut ancestry = Vec::new();
    let mut current = release.clone();
    loop {
        if ancestry.len() == MAX_RELEASE_LINEAGE_DEPTH {
            return Err(LocalPdmError::LineageTooDeep);
        }
        if current.document.document_id != document_id || ancestry.contains(&current.release_id) {
            return Err(LocalPdmError::InvalidManifest);
        }
        ancestry.push(current.release_id.clone());
        let Some(parent_release_id) = current.parent_release_id else {
            break;
        };
        current = read_manifest(repository, &parent_release_id)?;
    }
    Ok(ancestry)
}

fn read_manifest(repository: &Path, release_id: &str) -> Result<ReleaseManifest, LocalPdmError> {
    validate_sha256(release_id)?;
    let manifest_bytes = match read_bounded_file(
        &release_manifest_path(repository, release_id)?,
        MAX_RELEASE_MANIFEST_BYTES,
        LocalPdmError::ManifestTooLarge,
    ) {
        Err(LocalPdmError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(LocalPdmError::MissingRelease {
                release_id: release_id.to_owned(),
            });
        }
        result => result?,
    };
    let manifest: ReleaseManifest = serde_json::from_slice(&manifest_bytes)?;
    validate_manifest(&manifest, release_id)?;
    Ok(manifest)
}

pub fn release_manifest_path(
    repository: impl AsRef<Path>,
    release_id: &str,
) -> Result<PathBuf, LocalPdmError> {
    validate_sha256(release_id)?;
    Ok(repository
        .as_ref()
        .join("releases")
        .join(format!("{release_id}.json")))
}

pub fn release_object_path(
    repository: impl AsRef<Path>,
    sha256: &str,
) -> Result<PathBuf, LocalPdmError> {
    validate_sha256(sha256)?;
    Ok(repository
        .as_ref()
        .join("objects")
        .join(&sha256[..2])
        .join(sha256))
}

fn validate_manifest(
    manifest: &ReleaseManifest,
    expected_release_id: &str,
) -> Result<(), LocalPdmError> {
    if manifest.schema != LOCAL_PDM_RELEASE_SCHEMA_V1
        || manifest.release_id != expected_release_id
        || manifest.computed_release_id()? != expected_release_id
        || manifest.dependencies.len() > MAX_RELEASE_DEPENDENCIES
        || manifest.document.units != "millimetres"
    {
        return Err(LocalPdmError::ManifestIdentityMismatch);
    }
    if let Some(parent_release_id) = &manifest.parent_release_id {
        validate_sha256(parent_release_id)?;
        if parent_release_id == expected_release_id {
            return Err(LocalPdmError::InvalidManifest);
        }
    }
    validate_audit(&manifest.audit)?;
    validate_object_identity(&manifest.document.object)?;
    let mut previous = None;
    for dependency in &manifest.dependencies {
        validate_logical_path(&dependency.logical_path)?;
        validate_object_identity(&dependency.object)?;
        if previous.is_some_and(|path: &str| path >= dependency.logical_path.as_str()) {
            return Err(LocalPdmError::InvalidManifest);
        }
        previous = Some(dependency.logical_path.as_str());
    }
    Ok(())
}

fn validate_audit(audit: &ReleaseAudit) -> Result<(), LocalPdmError> {
    if audit.actor.trim() != audit.actor
        || audit.actor.is_empty()
        || audit.actor.len() > MAX_AUDIT_ACTOR_BYTES
        || audit.actor.chars().any(char::is_control)
        || audit.created_unix_ms == 0
        || audit.note.trim() != audit.note
        || audit.note.len() > MAX_AUDIT_NOTE_BYTES
        || audit.note.chars().any(char::is_control)
    {
        return Err(LocalPdmError::InvalidAudit);
    }
    Ok(())
}

fn validate_logical_path(path: &str) -> Result<(), LocalPdmError> {
    if path.is_empty()
        || path.len() > MAX_LOGICAL_PATH_BYTES
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return Err(LocalPdmError::InvalidLogicalPath);
    }
    let parsed = Path::new(path);
    if parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(LocalPdmError::InvalidLogicalPath);
    }
    Ok(())
}

fn validate_object_identity(identity: &ReleaseObjectIdentity) -> Result<(), LocalPdmError> {
    validate_sha256(&identity.sha256)?;
    if identity.byte_len > MAX_RELEASE_DEPENDENCY_BYTES as u64 {
        return Err(LocalPdmError::InvalidManifest);
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), LocalPdmError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LocalPdmError::InvalidReleaseId);
    }
    Ok(())
}

fn canonical_payload_identity(payload: &ReleasePayload) -> Result<String, LocalPdmError> {
    Ok(sha256_hex(&serde_json::to_vec(payload)?))
}

fn identity(bytes: &[u8]) -> ReleaseObjectIdentity {
    ReleaseObjectIdentity {
        sha256: sha256_hex(bytes),
        byte_len: bytes.len() as u64,
    }
}

fn read_bounded_file(
    path: &Path,
    limit: usize,
    limit_error: LocalPdmError,
) -> Result<Vec<u8>, LocalPdmError> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > limit as u64 {
        return Err(limit_error);
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(limit_error);
    }
    Ok(bytes)
}

fn write_content_addressed_object(
    repository: &Path,
    identity: &ReleaseObjectIdentity,
    bytes: &[u8],
) -> Result<(), LocalPdmError> {
    let path = release_object_path(repository, &identity.sha256)?;
    if path.exists() {
        return verify_existing_object(&path, identity, bytes);
    }
    let parent = path.parent().ok_or(LocalPdmError::InvalidManifest)?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    match temporary.persist_noclobber(&path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            verify_existing_object(&path, identity, bytes)
        }
        Err(error) => Err(LocalPdmError::Io(error.error)),
    }
}

fn verify_existing_object(
    path: &Path,
    identity: &ReleaseObjectIdentity,
    expected_bytes: &[u8],
) -> Result<(), LocalPdmError> {
    let actual = read_bounded_file(
        path,
        MAX_RELEASE_DEPENDENCY_BYTES,
        LocalPdmError::ObjectConflict {
            sha256: identity.sha256.clone(),
        },
    )?;
    if actual != expected_bytes || self::identity(&actual) != *identity {
        return Err(LocalPdmError::ObjectConflict {
            sha256: identity.sha256.clone(),
        });
    }
    Ok(())
}

fn write_manifest(repository: &Path, release_id: &str, bytes: &[u8]) -> Result<(), LocalPdmError> {
    let path = release_manifest_path(repository, release_id)?;
    let parent = path.parent().ok_or(LocalPdmError::InvalidManifest)?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            Err(LocalPdmError::ReleaseAlreadyExists)
        }
        Err(error) => Err(LocalPdmError::Io(error.error)),
    }
}

fn read_object(
    repository: &Path,
    expected: &ReleaseObjectIdentity,
) -> Result<Vec<u8>, LocalPdmError> {
    validate_object_identity(expected)?;
    let path = release_object_path(repository, &expected.sha256)?;
    let bytes = match read_bounded_file(
        &path,
        MAX_RELEASE_DEPENDENCY_BYTES,
        LocalPdmError::ObjectTampered {
            sha256: expected.sha256.clone(),
        },
    ) {
        Err(LocalPdmError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(LocalPdmError::MissingObject {
                sha256: expected.sha256.clone(),
            });
        }
        result => result?,
    };
    if identity(&bytes) != *expected {
        return Err(LocalPdmError::ObjectTampered {
            sha256: expected.sha256.clone(),
        });
    }
    Ok(bytes)
}
