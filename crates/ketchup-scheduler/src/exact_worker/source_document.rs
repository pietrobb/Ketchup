//! The imported STEP or IGES source a worker read last (review 2026-10-18 EX-8).
//!
//! Evaluating imported parts asks the worker to inspect a part, then to
//! tessellate it, part after part, and each request parsed the whole source
//! again: 300 full parses for a 100-part STEP on every reopen. Every request
//! about an XDE source now reads it here, and the worker keeps the document of
//! the last source it read, by format and SHA-256, so the requests that follow
//! about the same source are answered from that one parse. Only one is kept: a
//! request about another source releases it before reading that one.

use super::{ExactBackend, SourceFile, WorkerFailure, backend_failure, temp_path, verified_copy};
use ketchup_exact::StepXdeDocument;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum XdeFormat {
    Step,
    Iges,
}

impl XdeFormat {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Step => "STEP",
            Self::Iges => "IGES",
        }
    }
}

struct ReadSource {
    format: XdeFormat,
    sha256: String,
    byte_len: u64,
    document: StepXdeDocument,
}

#[derive(Default)]
pub(super) struct SourceDocument(Option<ReadSource>);

impl SourceDocument {
    /// The XDE document of `source` and its length in bytes; the source is read
    /// and verified against its SHA-256 unless it is the one read last.
    pub(super) fn get(
        &mut self,
        backend: &ExactBackend,
        source: &SourceFile,
        format: XdeFormat,
        operation: &'static str,
    ) -> Result<(&StepXdeDocument, u64), WorkerFailure> {
        let current = self
            .0
            .as_ref()
            .is_some_and(|read| read.format == format && read.sha256 == source.sha256);
        if !current {
            self.0 = None;
            let copy = verified_copy(source, format.label(), operation)?;
            let byte_len = copy
                .as_file()
                .metadata()
                .map_err(|error| backend_failure(operation, &error.to_string()))?
                .len();
            let path = temp_path(&copy);
            let document = match format {
                XdeFormat::Step => backend.open_step_xde(&path),
                XdeFormat::Iges => backend.open_iges_xde(&path),
            }
            .map_err(|error| backend_failure(operation, &error.to_string()))?;
            self.0 = Some(ReadSource {
                format,
                sha256: source.sha256.clone(),
                byte_len,
                document,
            });
        }
        let read = self.0.as_ref().expect("the source was read above");
        Ok((&read.document, read.byte_len))
    }
}
