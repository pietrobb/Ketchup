//! One save pipeline (review 2026-10-15 S2): [`prepare_document`] encodes a
//! document once, with one rule for an Undo history that does not fit, and
//! [`publish_document`] or [`publish_work_recovery`] write those bytes.
use std::path::Path;

use super::{
    ContainerData, FileIdentity, FilePersistenceError, PersistenceError, save_atomic_bytes,
    save_atomic_bytes_if_absent, save_container, save_document_store,
    save_document_store_current_snapshot, write_work_recovery,
};
use crate::document::{DocumentStore, Snapshot};

/// Which Undo history a prepared file carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryPolicy {
    /// The whole history; a history that does not fit is an error.
    Keep,
    /// The whole history, or only the current revision when the history does not fit.
    KeepIfFits,
    /// Only the current revision.
    CurrentOnly,
}

/// A native document file encoded by [`prepare_document`], ready to publish.
#[derive(Clone, Debug)]
pub struct PreparedDocument {
    bytes: Vec<u8>,
    identity: FileIdentity,
    keeps_history: bool,
}

impl PreparedDocument {
    fn new(bytes: Vec<u8>, keeps_history: bool) -> Self {
        Self {
            identity: FileIdentity::from_bytes(&bytes),
            bytes,
            keeps_history,
        }
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Whether the file carries the document's Undo history; without it, the
    /// history before the current revision is gone once the file is published.
    #[must_use]
    pub fn keeps_history(&self) -> bool {
        self.keeps_history
    }
}

/// Encodes `document` once as a native file under `policy`.
pub fn prepare_document(
    document: &DocumentStore,
    container: &ContainerData,
    policy: HistoryPolicy,
) -> Result<PreparedDocument, PersistenceError> {
    let current_only = || save_document_store_current_snapshot(document, container);
    let (bytes, keeps_history) = match policy {
        HistoryPolicy::Keep => (save_document_store(document, container)?, true),
        HistoryPolicy::CurrentOnly => (current_only()?, false),
        HistoryPolicy::KeepIfFits => match save_document_store(document, container) {
            Ok(bytes) => (bytes, true),
            Err(PersistenceError::ResourceLimit | PersistenceError::TooLarge { .. }) => {
                (current_only()?, false)
            }
            Err(error) => return Err(error),
        },
    };
    Ok(PreparedDocument::new(bytes, keeps_history))
}

/// Encodes one revision with no Undo history, e.g. a migrated or exported snapshot.
pub fn prepare_snapshot(
    snapshot: &Snapshot,
    container: &ContainerData,
) -> Result<PreparedDocument, PersistenceError> {
    save_container(snapshot, container).map(|bytes| PreparedDocument::new(bytes, false))
}

/// What publishing may replace at the destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Publish {
    /// Whatever the path holds; an intact replaced file is kept as its `.recovery`.
    Replace,
    /// Only the file with this identity; anything else is an external conflict.
    IfUnchanged(FileIdentity),
    /// Only a path that does not exist yet.
    IfAbsent,
}

impl Publish {
    /// Replace only the file observed with `existing`, or require none.
    #[must_use]
    pub fn over(existing: Option<FileIdentity>) -> Self {
        existing.map_or(Self::IfAbsent, Self::IfUnchanged)
    }
}

/// Writes `prepared` atomically at `path`.
pub fn publish_document(
    path: &Path,
    prepared: &PreparedDocument,
    mode: Publish,
) -> Result<(), FilePersistenceError> {
    match mode {
        Publish::Replace => save_atomic_bytes(path, prepared.bytes(), None),
        Publish::IfUnchanged(identity) => save_atomic_bytes(path, prepared.bytes(), Some(identity)),
        Publish::IfAbsent => save_atomic_bytes_if_absent(path, prepared.bytes()),
    }
}

/// Writes `prepared` as the work-recovery checkpoint of the file at `path`, bound
/// to that file's `base_identity`; it replaces only the checkpoint this writer
/// observed (`None` requires there is none). Live sessions must also hold a
/// `WorkRecoveryLock` to protect a recovery reopen.
pub fn publish_work_recovery(
    path: &Path,
    prepared: &PreparedDocument,
    base_identity: FileIdentity,
    expected_recovery: Option<FileIdentity>,
) -> Result<FileIdentity, FilePersistenceError> {
    write_work_recovery(
        path,
        prepared.bytes(),
        base_identity,
        expected_recovery,
        || {},
    )
}

/// Saves one revision over whatever `path` holds.
pub fn save_atomic(
    path: impl AsRef<Path>,
    snapshot: &Snapshot,
) -> Result<(), FilePersistenceError> {
    let prepared = prepare_snapshot(snapshot, &ContainerData::default())
        .map_err(FilePersistenceError::Format)?;
    publish_document(path.as_ref(), &prepared, Publish::Replace)
}
