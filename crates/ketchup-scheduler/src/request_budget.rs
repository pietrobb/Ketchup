//! How long the worker may take for one request: one table by the request's
//! kind and size, not one constant at every call site. Reading, transferring
//! and writing an exchange file (STEP, IGES) grows with its bytes and with the
//! parts it holds, so such a request is given time for both; cancellation
//! still ends it at once.

use crate::protocol::WorkerRequest;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The fixed part of an exchange request: starting the reader or writer.
const EXCHANGE_BASE: Duration = Duration::from_secs(15);
/// Per started MiB of the files an exchange request reads.
const EXCHANGE_PER_MIB: Duration = Duration::from_secs(4);
/// Per part an exchange request transfers on its own.
const EXCHANGE_PER_PART: Duration = Duration::from_secs(2);
/// No exchange request is given longer.
const EXCHANGE_MOST: Duration = Duration::from_secs(15 * 60);

/// The time an exchange request reading `bytes` and transferring `parts` parts is given.
#[must_use]
pub(crate) fn exchange_budget(bytes: u64, parts: usize) -> Duration {
    let mib = u32::try_from(bytes.div_ceil(1 << 20)).unwrap_or(u32::MAX);
    let parts = u32::try_from(parts).unwrap_or(u32::MAX);
    EXCHANGE_BASE
        .saturating_add(EXCHANGE_PER_MIB.saturating_mul(mib))
        .saturating_add(EXCHANGE_PER_PART.saturating_mul(parts))
        .min(EXCHANGE_MOST)
}

/// Bytes of the files at `paths`; a file that cannot be read adds nothing,
/// as the worker then refuses it at once.
fn bytes_of<'a>(paths: impl IntoIterator<Item = &'a Path>) -> u64 {
    paths
        .into_iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum()
}

impl WorkerRequest {
    /// The time this request is given, or `None` when its caller sets it.
    #[must_use]
    pub(crate) fn exchange_budget(&self) -> Option<Duration> {
        let (bytes, parts) = match self {
            Self::InspectStepXde(source) | Self::InspectIgesXde(source) => {
                (bytes_of([source.path.as_path()]), 0)
            }
            Self::InspectStepPart { source, .. }
            | Self::InspectIgesPart { source, .. }
            | Self::TessellateStepPart { source, .. }
            | Self::TessellateIgesPart { source, .. }
            | Self::ExportStepXdePart { source, .. } => (bytes_of([source.path.as_path()]), 1),
            Self::ConvertStepXdeToIges {
                source,
                occurrences,
                ..
            } => (bytes_of([source.path.as_path()]), *occurrences),
            Self::AssembleStep {
                manifest, sources, ..
            } => (
                bytes_of(sources.iter().map(PathBuf::as_path)),
                manifest.parts.len(),
            ),
            _ => return None,
        };
        Some(exchange_budget(bytes, parts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exchange_request_is_given_time_for_its_bytes_and_parts_within_a_bound() {
        assert_eq!(exchange_budget(0, 0), EXCHANGE_BASE);
        // A 30 MiB STEP with 100 parts: far beyond the 5 s of a simple call.
        assert_eq!(
            exchange_budget(30 << 20, 100),
            Duration::from_secs(15 + 4 * 30 + 2 * 100)
        );
        assert!(exchange_budget(1, 1) > exchange_budget(0, 1));
        assert_eq!(exchange_budget(u64::MAX, usize::MAX), EXCHANGE_MOST);
    }

    /// Review 2026-10-18 EX-9: converting an assembly of 750 occurrences (10 MiB
    /// STEP) to IGES was given 57 s, as for one part, and timed out.
    #[test]
    fn an_iges_conversion_is_given_time_for_every_occurrence() {
        let source = tempfile::NamedTempFile::new().unwrap();
        source.as_file().set_len(10 << 20).unwrap();
        let request = WorkerRequest::ConvertStepXdeToIges {
            source: crate::protocol::SourceFile {
                sha256: "a".repeat(64),
                path: source.path().to_owned(),
            },
            output: PathBuf::from("model.iges"),
            occurrences: 750,
        };
        assert_eq!(
            request.exchange_budget(),
            Some(exchange_budget(10 << 20, 750))
        );
        assert_eq!(request.exchange_budget(), Some(EXCHANGE_MOST));
    }
}
