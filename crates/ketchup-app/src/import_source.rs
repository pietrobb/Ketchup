//! One sealed import source shared by every import format: the bounded read, the seal taken
//! when the user picks the file, and the checks that make an import publish exactly what the
//! user reviewed. Every failure names its format and keeps its cause.

use super::*;

/// The picked source bytes sealed against the document revision they were reviewed for.
/// `unit` carries the format's own review choice (the declared length unit for STL and DXF).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImportSourcePlan<U = ()> {
    pub(crate) format: ImportFormat,
    pub(crate) path: PathBuf,
    pub(crate) source: Vec<u8>,
    pub(crate) unit: U,
    pub(crate) document_id: DocumentId,
    pub(crate) revision_id: u64,
    pub(crate) canonical_digest: String,
    pub(crate) source_sha256: [u8; 32],
    pub(crate) source_byte_len: u64,
}

impl<U> ImportSourcePlan<U> {
    pub(crate) fn seal(
        format: ImportFormat,
        path: PathBuf,
        source: Vec<u8>,
        unit: U,
        snapshot: &Snapshot,
    ) -> Self {
        Self {
            format,
            path,
            source_sha256: sha256_bytes(&source),
            source_byte_len: source.len() as u64,
            source,
            unit,
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
        }
    }

    pub(crate) fn error(&self, failure: ImportFailure) -> ImportError {
        ImportError {
            format: self.format,
            failure,
        }
    }

    pub(crate) fn failed(&self, cause: impl Into<BoxedCause>) -> ImportError {
        self.error(ImportFailure::Failed(cause.into()))
    }

    /// Requires the seal to still describe the active document and its own bytes; returns the
    /// source file name every planner records in the import receipt.
    pub(crate) fn verify_seal(&self, snapshot: &Snapshot) -> Result<&str, ImportError> {
        if snapshot.document_id() != self.document_id
            || snapshot.revision_id() != self.revision_id
            || snapshot.canonical_digest() != self.canonical_digest
        {
            return Err(self.error(ImportFailure::ReviewStale));
        }
        if self.source.len() as u64 != self.source_byte_len
            || sha256_bytes(&self.source) != self.source_sha256
        {
            return Err(self.error(ImportFailure::SealBroken));
        }
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| self.error(ImportFailure::SourceNameNotUtf8))
    }

    /// Reads the file again at publication and requires the reviewed bytes.
    pub(crate) fn reread_unchanged(&self) -> Result<Vec<u8>, ImportError> {
        let source = read_import_source(self.format, &self.path)?;
        if source.len() as u64 != self.source_byte_len
            || sha256_bytes(&source) != self.source_sha256
            || source != self.source
        {
            return Err(self.error(ImportFailure::SourceChanged));
        }
        Ok(source)
    }
}

/// Reads at most the format's bounded envelope, refusing larger files before and after reading.
pub(crate) fn read_import_source(
    format: ImportFormat,
    path: &Path,
) -> Result<Vec<u8>, ImportError> {
    let limit_bytes = format.source_limit_bytes();
    let failed = |cause: std::io::Error| ImportError {
        format,
        failure: ImportFailure::Failed(cause.into()),
    };
    let too_large = ImportError {
        format,
        failure: ImportFailure::SourceTooLarge { limit_bytes },
    };
    if std::fs::metadata(path).map_err(failed)?.len() > limit_bytes {
        return Err(too_large);
    }
    let mut file = std::fs::File::open(path).map_err(failed)?;
    let mut source = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(limit_bytes + 1)
        .read_to_end(&mut source)
        .map_err(failed)?;
    if source.len() as u64 > limit_bytes {
        return Err(too_large);
    }
    Ok(source)
}

pub(crate) type BoxedCause = Box<dyn std::error::Error + Send + Sync>;

/// Why one import of one format was refused.
#[derive(Debug)]
pub(crate) struct ImportError {
    pub(crate) format: ImportFormat,
    pub(crate) failure: ImportFailure,
}

impl ImportError {
    pub(crate) fn failed(format: ImportFormat, cause: impl Into<BoxedCause>) -> Self {
        Self {
            format,
            failure: ImportFailure::Failed(cause.into()),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ImportFailure {
    SourceTooLarge {
        limit_bytes: u64,
    },
    /// The document changed after the review was prepared, or the review was invalidated.
    ReviewStale,
    /// The review still reports an error or awaits a required confirmation.
    ReviewIncomplete,
    /// The sealed bytes no longer match their recorded identity.
    SealBroken,
    SourceNameNotUtf8,
    /// The file on disk differs from the bytes that were reviewed.
    SourceChanged,
    /// Re-deriving the review, proposal or stored source no longer gives what was previewed.
    ReviewChanged,
    /// A reader, parser, worker or document step refused; its error is the cause.
    Failed(BoxedCause),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format = self.format;
        match &self.failure {
            ImportFailure::SourceTooLarge { limit_bytes } => write!(
                formatter,
                "{format} source exceeds the bounded {limit_bytes}-byte envelope"
            ),
            ImportFailure::ReviewStale => {
                write!(
                    formatter,
                    "{format} import review is stale for the active document"
                )
            }
            ImportFailure::ReviewIncomplete => {
                write!(formatter, "{format} import review is incomplete")
            }
            ImportFailure::SealBroken => {
                write!(
                    formatter,
                    "sealed {format} source identity does not match its bytes"
                )
            }
            ImportFailure::SourceNameNotUtf8 => {
                write!(formatter, "{format} source name is not valid UTF-8")
            }
            ImportFailure::SourceChanged => write!(
                formatter,
                "{format} source changed after it was selected for review"
            ),
            ImportFailure::ReviewChanged => write!(
                formatter,
                "{format} review, proposal or stored source changed after preview"
            ),
            ImportFailure::Failed(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.failure {
            ImportFailure::Failed(cause) => cause.source(),
            _ => None,
        }
    }
}

impl KetchupApp {
    /// Reports one finished import in the action digest; a refusal names its typed reason.
    pub(crate) fn report_import_outcome(
        &mut self,
        source: &ImportSourcePlan<impl Sized>,
        result: Result<(), ImportError>,
    ) -> bool {
        match result {
            Ok(()) => {
                self.digest = self.catalog.format(
                    &format!("digest-imported-{}", import_catalog_key(source.format)),
                    &BTreeMap::from([("path", source.path.display().to_string())]),
                );
                true
            }
            Err(error) => {
                self.report_import_failure(&source.path, &error);
                false
            }
        }
    }

    pub(crate) fn report_import_failure(&mut self, path: &Path, error: &ImportError) {
        self.digest = self.catalog.format(
            &format!("error-import-{}", import_catalog_key(error.format)),
            &BTreeMap::from([
                ("path", path.display().to_string()),
                ("reason", error.to_string()),
            ]),
        );
    }
}

const fn import_catalog_key(format: ImportFormat) -> &'static str {
    match format {
        ImportFormat::Stl => "stl",
        ImportFormat::Dxf => "dxf",
        ImportFormat::Step => "step",
        ImportFormat::Iges => "iges",
        ImportFormat::SketchupScene => "sketchup-scene",
        ImportFormat::Glb => "glb",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_import_refusal_names_its_format_its_typed_reason_and_keeps_its_cause() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("part.glb");
        std::fs::write(&path, b"reviewed").unwrap();
        let snapshot = DocumentStore::new().current();
        let source = read_import_source(ImportFormat::Glb, &path).unwrap();
        let plan = ImportSourcePlan::seal(ImportFormat::Glb, path.clone(), source, (), &snapshot);
        assert_eq!(plan.verify_seal(&snapshot).unwrap(), "part.glb");
        assert_eq!(plan.reread_unchanged().unwrap(), b"reviewed");

        let mut stale = plan.clone();
        stale.revision_id += 1;
        let error = stale.verify_seal(&snapshot).unwrap_err();
        assert!(matches!(error.failure, ImportFailure::ReviewStale));
        assert_eq!(
            error.to_string(),
            "GLB import review is stale for the active document"
        );

        let mut broken = plan.clone();
        broken.source.push(0);
        let error = broken.verify_seal(&snapshot).unwrap_err();
        assert!(matches!(error.failure, ImportFailure::SealBroken));

        std::fs::write(&path, b"replaced").unwrap();
        let error = plan.reread_unchanged().unwrap_err();
        assert!(matches!(error.failure, ImportFailure::SourceChanged));
        assert_eq!(
            error.to_string(),
            "GLB source changed after it was selected for review"
        );

        let large = directory.path().join("large.dxf");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(ImportFormat::Dxf.source_limit_bytes() + 1)
            .unwrap();
        let error = read_import_source(ImportFormat::Dxf, &large).unwrap_err();
        assert!(matches!(
            error.failure,
            ImportFailure::SourceTooLarge { limit_bytes }
                if limit_bytes == ImportFormat::Dxf.source_limit_bytes()
        ));
        assert!(
            error
                .to_string()
                .starts_with("DXF source exceeds the bounded")
        );

        let error = read_import_source(ImportFormat::Step, &directory.path().join("missing.step"))
            .unwrap_err();
        let ImportFailure::Failed(cause) = &error.failure else {
            panic!("a missing file must keep its I/O error, got {error:?}");
        };
        assert!(cause.downcast_ref::<std::io::Error>().is_some());
    }
}
