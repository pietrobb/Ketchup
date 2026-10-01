//! Migrating a review-only document into a new editable copy: the review is sealed against the
//! exact loaded bytes and re-derived before anything is written. Every refusal is typed.

use super::*;

/// Why a review-only document could not be migrated into a new copy.
#[derive(Debug)]
pub(crate) enum MigrationError {
    /// No review-only migration candidate is loaded or pending.
    NotReviewCandidate,
    /// Re-deriving the review from the source bytes no longer gives the sealed review.
    ReviewMismatch,
    DestinationNotNewCopy,
    DestinationExists,
    ActiveDocumentChanged,
    /// The sealed bytes no longer match their recorded identity.
    SealBroken,
    /// The file on disk differs from the bytes that were reviewed.
    SourceChanged,
    /// Loading, migrating, encoding or writing refused; its error is the cause.
    Failed(import_source::BoxedCause),
}

impl MigrationError {
    fn failed(cause: impl Into<import_source::BoxedCause>) -> Self {
        Self::Failed(cause.into())
    }
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotReviewCandidate => "no review-only migration candidate is pending",
            Self::ReviewMismatch => {
                "the migration review no longer matches the sealed source bytes"
            }
            Self::DestinationNotNewCopy => "migration destination must be a new copy",
            Self::DestinationExists => "migration destination already exists",
            Self::ActiveDocumentChanged => "active document changed after migration review",
            Self::SealBroken => "sealed migration source identity does not match its bytes",
            Self::SourceChanged => "migration source changed after review",
            Self::Failed(cause) => return cause.fmt(formatter),
        })
    }
}

impl std::error::Error for MigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Failed(cause) => cause.source(),
            _ => None,
        }
    }
}

impl KetchupApp {
    fn migration_review_identity(
        outcome: &ketchup_model::persistence::LoadOutcome,
    ) -> Result<MigrationReviewIdentity, MigrationError> {
        let candidate = outcome
            .review_candidate()
            .ok_or(MigrationError::NotReviewCandidate)?;
        let snapshot = candidate.snapshot();
        let mut audit = candidate.audit().clone();
        audit.recovered_from_backup = false;
        let container =
            ketchup_model::persistence::save_container(snapshot, candidate.container_data())
                .map_err(MigrationError::failed)?;
        Ok(MigrationReviewIdentity {
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            audit,
            container_sha256: sha256_bytes(&container),
        })
    }

    pub(super) fn prepare_migration_review_plan(
        &self,
        path: &Path,
        effective_path: PathBuf,
        source: Vec<u8>,
        outcome: &ketchup_model::persistence::LoadOutcome,
    ) -> Result<MigrationReviewPlan, MigrationError> {
        let rederived =
            ketchup_model::persistence::load(&source).map_err(MigrationError::failed)?;
        let review = Self::migration_review_identity(outcome)?;
        if Self::migration_review_identity(&rederived)? != review {
            return Err(MigrationError::ReviewMismatch);
        }
        let active = self.document.current();
        Ok(MigrationReviewPlan {
            source: MigrationReviewSourcePlan {
                path: path.to_owned(),
                effective_path,
                source_sha256: sha256_bytes(&source),
                source_byte_len: source.len() as u64,
                source,
                active_document_id: active.document_id(),
                active_revision_id: active.revision_id(),
                active_canonical_digest: active.canonical_digest(),
            },
            review,
        })
    }

    pub fn confirm_review_candidate_migration_to(&mut self, destination: &Path) -> bool {
        let Some(plan) = self.file.migration_review_plan.clone() else {
            return false;
        };
        let result = (|| {
            let comparable_path = |path: &Path| {
                std::fs::canonicalize(path).unwrap_or_else(|_| {
                    path.parent()
                        .and_then(|parent| std::fs::canonicalize(parent).ok())
                        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
                        .unwrap_or_else(|| path.to_owned())
                })
            };
            if comparable_path(&plan.source.path) == comparable_path(destination)
                || comparable_path(&plan.source.effective_path) == comparable_path(destination)
            {
                return Err(MigrationError::DestinationNotNewCopy);
            }
            if destination.exists() {
                return Err(MigrationError::DestinationExists);
            }

            let active = self.document.current();
            if active.document_id() != plan.source.active_document_id
                || active.revision_id() != plan.source.active_revision_id
                || active.canonical_digest() != plan.source.active_canonical_digest
            {
                return Err(MigrationError::ActiveDocumentChanged);
            }
            let pending = self
                .file
                .review_candidate
                .as_ref()
                .ok_or(MigrationError::NotReviewCandidate)?;
            if Self::migration_review_identity(pending)? != plan.review {
                return Err(MigrationError::ReviewMismatch);
            }
            if plan.source.source.len() as u64 != plan.source.source_byte_len
                || sha256_bytes(&plan.source.source) != plan.source.source_sha256
            {
                return Err(MigrationError::SealBroken);
            }
            let current_source =
                ketchup_model::persistence::read_native_document_file(&plan.source.effective_path)
                    .map_err(MigrationError::failed)?;
            if current_source != plan.source.source
                || current_source.len() as u64 != plan.source.source_byte_len
                || sha256_bytes(&current_source) != plan.source.source_sha256
            {
                return Err(MigrationError::SourceChanged);
            }

            let rederived = ketchup_model::persistence::load(&current_source)
                .map_err(MigrationError::failed)?;
            if Self::migration_review_identity(&rederived)? != plan.review {
                return Err(MigrationError::ReviewMismatch);
            }
            let confirmed = rederived
                .review_candidate()
                .ok_or(MigrationError::NotReviewCandidate)?
                .confirm_semantic_migration()
                .map_err(MigrationError::failed)?;
            let (document, container_data) = confirmed.into_parts();
            let snapshot = document.current();
            let bytes = ketchup_model::persistence::save_container(&snapshot, &container_data)
                .map_err(MigrationError::failed)?;
            let parent = destination.parent().unwrap_or_else(|| Path::new("."));
            let mut temporary =
                tempfile::NamedTempFile::new_in(parent).map_err(MigrationError::failed)?;
            temporary
                .write_all(&bytes)
                .and_then(|()| temporary.as_file_mut().sync_all())
                .map_err(MigrationError::failed)?;
            temporary
                .persist_noclobber(destination)
                .map_err(|error| MigrationError::failed(error.error))?;
            let file_identity = ketchup_model::persistence::FileIdentity::from_bytes(&bytes);
            Ok((document, container_data, file_identity))
        })();
        let (mut document, container_data, file_identity) = match result {
            Ok(confirmed) => confirmed,
            Err(reason) => {
                self.digest = self.catalog.format(
                    "error-migrate-document",
                    &BTreeMap::from([("reason", reason.to_string())]),
                );
                return false;
            }
        };

        document.discard_history_before_current();
        let saved_digest = document.history_digest();
        self.document = document;
        self.file.container_data = container_data;
        self.file.review_candidate = None;
        self.file.migration_review_plan = None;
        self.file.recovery_open = None;
        self.file.path = Some(destination.to_owned());
        self.file.identity = Some(file_identity);
        self.file.work_recovery_identity = None;
        self.file.work_recovery_digest = None;
        self.file.saved_digest = saved_digest;
        self.reset_document_presentation();
        self.digest = self.catalog.format(
            "digest-migrated-document",
            &BTreeMap::from([("path", destination.display().to_string())]),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_refusals_name_their_typed_reason() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("legacy-review.ketchup");
        let occupied = directory.path().join("occupied.ketchup");
        let destination = directory.path().join("migrated.ketchup");
        let source_bytes = crate::tests::lossy_legacy_document();
        std::fs::write(&source, &source_bytes).unwrap();
        std::fs::write(&occupied, b"keep").unwrap();
        let mut app = KetchupApp::new();
        assert!(!app.open_document_from(&source));
        assert!(app.has_review_candidate());

        for (target, reason) in [
            (&source, MigrationError::DestinationNotNewCopy),
            (&occupied, MigrationError::DestinationExists),
        ] {
            assert!(!app.confirm_review_candidate_migration_to(target));
            assert!(
                app.action_digest().contains(&reason.to_string()),
                "{}",
                app.action_digest()
            );
        }
        std::fs::write(&source, b"tampered after review").unwrap();
        assert!(!app.confirm_review_candidate_migration_to(&destination));
        assert!(
            app.action_digest()
                .contains(&MigrationError::SourceChanged.to_string()),
            "{}",
            app.action_digest()
        );
        assert!(!destination.exists());
        assert!(app.has_review_candidate());
    }
}
