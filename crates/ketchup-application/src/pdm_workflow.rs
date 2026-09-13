use ketchup_core::document::Snapshot;
use ketchup_core::local_pdm::{
    LocalPdmError, ReleaseAudit, ReleaseCatalogEntry, ReleaseComparison, ReleaseDependencyInput,
    ReleaseManifest, VerifiedRelease, compare_releases, create_child_release, create_release,
    open_release, release_catalog,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdmSourceIdentity {
    pub revision: u64,
    pub canonical_digest: String,
}

impl PdmSourceIdentity {
    #[must_use]
    pub fn observed(snapshot: &Snapshot) -> Self {
        Self {
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
        }
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
        snapshot: &Snapshot,
        source: &PdmSourceIdentity,
        request: &PdmCreateReleaseRequest,
        confirmed: bool,
        cancelled: &AtomicBool,
    ) -> Result<ReleaseManifest, PdmWorkflowError> {
        if !confirmed {
            return Err(PdmWorkflowError::ConfirmationRequired);
        }
        ensure_current(snapshot, source)?;
        ensure_not_cancelled(cancelled)?;
        let manifest = match request.parent_release_id.as_deref() {
            Some(parent_release_id) => create_child_release(
                &request.repository,
                parent_release_id,
                snapshot,
                source.revision,
                &source.canonical_digest,
                &request.dependencies,
                request.audit.clone(),
            ),
            None => create_release(
                &request.repository,
                snapshot,
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
        snapshot: &Snapshot,
        source: &PdmSourceIdentity,
        repository: impl AsRef<Path>,
        release_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<VerifiedRelease, PdmWorkflowError> {
        ensure_current(snapshot, source)?;
        ensure_not_cancelled(cancelled)?;
        let release = open_release(repository, release_id)?;
        ensure_not_cancelled(cancelled)?;
        Ok(release)
    }

    pub fn catalog(
        &self,
        snapshot: &Snapshot,
        source: &PdmSourceIdentity,
        repository: impl AsRef<Path>,
        cancelled: &AtomicBool,
    ) -> Result<Vec<ReleaseCatalogEntry>, PdmWorkflowError> {
        ensure_current(snapshot, source)?;
        ensure_not_cancelled(cancelled)?;
        let catalog = release_catalog(repository)?;
        ensure_not_cancelled(cancelled)?;
        Ok(catalog)
    }

    pub fn compare(
        &self,
        snapshot: &Snapshot,
        source: &PdmSourceIdentity,
        repository: impl AsRef<Path>,
        left_release_id: &str,
        right_release_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<ReleaseComparison, PdmWorkflowError> {
        ensure_current(snapshot, source)?;
        ensure_not_cancelled(cancelled)?;
        let comparison = compare_releases(repository, left_release_id, right_release_id)?;
        ensure_not_cancelled(cancelled)?;
        Ok(comparison)
    }
}

fn ensure_current(snapshot: &Snapshot, source: &PdmSourceIdentity) -> Result<(), PdmWorkflowError> {
    if snapshot.revision_id() != source.revision
        || snapshot.canonical_digest() != source.canonical_digest
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
    fn observation_and_cancellation_are_fail_closed() {
        let snapshot = DocumentStore::new().current();
        let source = PdmSourceIdentity::observed(&snapshot);
        let cancelled = AtomicBool::new(true);
        let error = LocalPdmWorkflow::new()
            .catalog(&snapshot, &source, ".", &cancelled)
            .unwrap_err();
        assert!(matches!(error, PdmWorkflowError::Cancelled));

        let stale = PdmSourceIdentity {
            revision: source.revision + 1,
            canonical_digest: source.canonical_digest,
        };
        let error = LocalPdmWorkflow::new()
            .catalog(&snapshot, &stale, ".", &AtomicBool::new(false))
            .unwrap_err();
        assert!(matches!(error, PdmWorkflowError::StaleState));
    }
}
