//! One save pipeline (review 2026-10-15 S2): [`prepare_document`] encodes a
//! document's current revision once, and [`publish_document`] or [`publish_work_recovery`] write those bytes.
use std::path::Path;

use super::{
    ContainerData, FileIdentity, FilePersistenceError, PersistenceError, publish_bytes,
    save_container, save_document_store_current_snapshot, sync_parent_directory,
    work_recovery::write_work_recovery,
};
use crate::document::{DocumentStore, Snapshot};

/// A native document file encoded by [`prepare_document`], ready to publish.
#[derive(Clone, Debug)]
pub struct PreparedDocument {
    bytes: Vec<u8>,
}

impl PreparedDocument {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn identity(&self) -> FileIdentity {
        FileIdentity::from_bytes(&self.bytes)
    }
}

/// Encodes the current revision of `document` once as a native file. The Undo
/// history stays in memory: a file holds none, only the named checkpoint of
/// the revision it saves.
pub fn prepare_document(
    document: &DocumentStore,
    container: &ContainerData,
) -> Result<PreparedDocument, PersistenceError> {
    Ok(PreparedDocument {
        bytes: save_document_store_current_snapshot(document, container)?,
    })
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
    publish_bytes(path, prepared.bytes(), mode, || {}, sync_parent_directory)
}

/// Writes `document` with its Undo history, where it fits, as the work-recovery
/// checkpoint of the file at `path`, bound to that file's `base_identity`; it
/// replaces only the checkpoint this writer observed (`None` requires there is
/// none). Live sessions must also hold a `WorkRecoveryLock` to protect a recovery
/// reopen. Over a checkpoint it observed, it appends only the step from that
/// checkpoint to `document` where a step can name it (idea #49).
pub fn publish_work_recovery(
    path: &Path,
    document: &DocumentStore,
    container: &ContainerData,
    base_identity: FileIdentity,
    expected_recovery: Option<FileIdentity>,
) -> Result<FileIdentity, FilePersistenceError> {
    write_work_recovery(
        path,
        document,
        container,
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
    let bytes = save_container(snapshot, &ContainerData::default())
        .map_err(FilePersistenceError::Format)?;
    publish_bytes(
        path.as_ref(),
        &bytes,
        Publish::Replace,
        || {},
        sync_parent_directory,
    )
}
