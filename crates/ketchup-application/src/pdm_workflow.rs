use ketchup_core::document::Snapshot;
use ketchup_core::local_pdm::{
    LocalPdmError, ReleaseAudit, ReleaseCatalogEntry, ReleaseComparison, ReleaseDependencyInput,
    ReleaseManifest, VerifiedRelease, compare_releases, create_child_release_with_container,
    create_release_with_container, open_release, release_catalog,
};
use ketchup_core::persistence::ContainerData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdmSourceIdentity {
    pub revision: u64,
    pub canonical_digest: String,
    pub mutation_epoch: u64,
}

impl PdmSourceIdentity {
    #[must_use]
    pub fn observed(snapshot: &Snapshot, mutation_epoch: u64) -> Self {
        Self {
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            mutation_epoch,
        }
    }
}

#[derive(Clone)]
pub struct PdmDocumentState {
    snapshot: Snapshot,
    mutation_epoch: u64,
}

impl PdmDocumentState {
    #[must_use]
    pub fn observed(snapshot: Snapshot, mutation_epoch: u64) -> Self {
        Self {
            snapshot,
            mutation_epoch,
        }
    }

    #[must_use]
    pub fn source_identity(&self) -> PdmSourceIdentity {
        PdmSourceIdentity::observed(&self.snapshot, self.mutation_epoch)
    }

    #[must_use]
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdmCreateReleaseRequest {
    pub repository: PathBuf,
    pub parent_release_id: Option<String>,
    pub dependencies: Vec<ReleaseDependencyInput>,
    pub audit: ReleaseAudit,
}

#[derive(Debug)]
pub enum PdmWorkflowError {
    ConfirmationRequired,
    Cancelled,
    StaleState,
    Core(LocalPdmError),
}

impl std::fmt::Display for PdmWorkflowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmationRequired => formatter
                .write_str("creating an immutable PDM release requires explicit confirmation"),
            Self::Cancelled => formatter.write_str("local PDM operation was cancelled"),
            Self::StaleState => {
                formatter.write_str("local PDM request no longer matches the current document")
            }
            Self::Core(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PdmWorkflowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Core(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LocalPdmError> for PdmWorkflowError {
    fn from(error: LocalPdmError) -> Self {
        match error {
            LocalPdmError::StaleSnapshot => Self::StaleState,
            error => Self::Core(error),
        }
    }
}

#[derive(Default)]
pub struct LocalPdmWorkflow;

impl LocalPdmWorkflow {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn create(
        &self,
        current: &PdmDocumentState,
        container_data: &ContainerData,
        source: &PdmSourceIdentity,
        request: &PdmCreateReleaseRequest,
        confirmed: bool,
        cancelled: &AtomicBool,
    ) -> Result<ReleaseManifest, PdmWorkflowError> {
        if !confirmed {
            return Err(PdmWorkflowError::ConfirmationRequired);
        }
        ensure_current(current, source)?;
        ensure_not_cancelled(cancelled)?;
        let snapshot = &current.snapshot;
        let manifest = match request.parent_release_id.as_deref() {
            Some(parent_release_id) => create_child_release_with_container(
                &request.repository,
                parent_release_id,
                (snapshot, container_data),
                source.revision,
                &source.canonical_digest,
                &request.dependencies,
                request.audit.clone(),
            ),
            None => create_release_with_container(
                &request.repository,
                (snapshot, container_data),
                source.revision,
                &source.canonical_digest,
                &request.dependencies,
                request.audit.clone(),
            ),
        }?;
        ensure_not_cancelled(cancelled)?;
        Ok(manifest)
    }

    pub fn open(
        &self,
        current: &PdmDocumentState,
        source: &PdmSourceIdentity,
        repository: impl AsRef<Path>,
        release_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<VerifiedRelease, PdmWorkflowError> {
        ensure_current(current, source)?;
        ensure_not_cancelled(cancelled)?;
        let release = open_release(repository, release_id)?;
        ensure_not_cancelled(cancelled)?;
        Ok(release)
    }

    pub fn catalog(
        &self,
        current: &PdmDocumentState,
        source: &PdmSourceIdentity,
        repository: impl AsRef<Path>,
        cancelled: &AtomicBool,
    ) -> Result<Vec<ReleaseCatalogEntry>, PdmWorkflowError> {
        ensure_current(current, source)?;
        ensure_not_cancelled(cancelled)?;
        let catalog = release_catalog(repository)?;
        ensure_not_cancelled(cancelled)?;
        Ok(catalog)
    }

    pub fn compare(
        &self,
        current: &PdmDocumentState,
        source: &PdmSourceIdentity,
        repository: impl AsRef<Path>,
        left_release_id: &str,
        right_release_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<ReleaseComparison, PdmWorkflowError> {
        ensure_current(current, source)?;
        ensure_not_cancelled(cancelled)?;
        let comparison = compare_releases(repository, left_release_id, right_release_id)?;
        ensure_not_cancelled(cancelled)?;
        Ok(comparison)
    }
}

fn ensure_current(
    current: &PdmDocumentState,
    source: &PdmSourceIdentity,
) -> Result<(), PdmWorkflowError> {
    if current.snapshot.revision_id() != source.revision
        || current.snapshot.canonical_digest() != source.canonical_digest
        || current.mutation_epoch != source.mutation_epoch
    {
        return Err(PdmWorkflowError::StaleState);
    }
    Ok(())
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<(), PdmWorkflowError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(PdmWorkflowError::Cancelled);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_core::document::DocumentStore;

    #[test]
    fn undo_aba_cannot_revive_reviewed_release_authority() {
        use ketchup_core::document::{CanonicalCommand, CommandBatch, DefinitionId};

        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Reviewed part".into(),
                },
            ]))
            .unwrap();
        let reviewed = document.current();
        let reviewed_epoch = document.mutation_epoch();
        let source = PdmSourceIdentity::observed(&reviewed, reviewed_epoch);

        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Changed part".into(),
                },
            ]))
            .unwrap();
        document.undo().unwrap();
        let restored = document.current();
        assert_eq!(restored.revision_id(), reviewed.revision_id());
        assert_eq!(restored.canonical_digest(), reviewed.canonical_digest());
        assert_ne!(document.mutation_epoch(), reviewed_epoch);

        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("repository");
        let current = PdmDocumentState::observed(restored, document.mutation_epoch());
        let result = LocalPdmWorkflow::new().create(
            &current,
            &ContainerData::default(),
            &source,
            &PdmCreateReleaseRequest {
                repository: repository.clone(),
                parent_release_id: None,
                dependencies: Vec::new(),
                audit: ReleaseAudit::new("reviewer", 1, "approved before Undo ABA"),
            },
            true,
            &AtomicBool::new(false),
        );
        assert!(matches!(result, Err(PdmWorkflowError::StaleState)));
        assert!(!repository.exists());
    }

    #[test]
    fn observation_and_cancellation_are_fail_closed() {
        let document = DocumentStore::new();
        let current = PdmDocumentState::observed(document.current(), document.mutation_epoch());
        let source = current.source_identity();
        let cancelled = AtomicBool::new(true);
        let error = LocalPdmWorkflow::new()
            .catalog(&current, &source, ".", &cancelled)
            .unwrap_err();
        assert!(matches!(error, PdmWorkflowError::Cancelled));

        let stale = PdmSourceIdentity {
            revision: source.revision + 1,
            canonical_digest: source.canonical_digest,
            mutation_epoch: source.mutation_epoch,
        };
        let error = LocalPdmWorkflow::new()
            .catalog(&current, &stale, ".", &AtomicBool::new(false))
            .unwrap_err();
        assert!(matches!(error, PdmWorkflowError::StaleState));
    }
}
