//! Crash-safe publication of export artifacts: one artifact or an artifact with its companion
//! report is published only over the bytes the human authorized, through a recovery journal
//! that a later export rolls forward or back. Every refusal is an `ExportTransactionError`
//! naming the path and keeping its I/O cause; every export command reports an `ExportError`.

use super::*;
use import_source::BoxedCause;

const EXPORT_BUNDLE_JOURNAL_SCHEMA_V1: &str = "ketchup.export-bundle-journal.v1";
const MAX_EXPORT_BUNDLE_JOURNAL_BYTES: u64 = 16 * 1024;

/// Why publishing or recovering export files was refused.
#[derive(Debug)]
pub(crate) enum ExportTransactionError {
    /// The file no longer holds the bytes the human authorized or the transaction wrote.
    TargetChanged { path: PathBuf },
    /// The target changed while it was being replaced; the authorized original was kept.
    OriginalPreserved { path: PathBuf, preserved: PathBuf },
    /// A target is a directory or another non-regular file.
    NotRegularFile { path: PathBuf },
    /// The artifact and its companion report are one file or live in different directories.
    PairNotSiblings,
    /// A transaction path has no file name, or one that is not valid UTF-8.
    UnnamedPath { path: PathBuf },
    /// The recovery journal cannot be trusted, so recovery touched nothing.
    InvalidJournal(JournalDefect),
    /// Recovery needs the backup of `path`, and it is missing or no longer holds the original.
    BackupMissing { path: PathBuf },
    /// A file system step on `path` failed.
    Io {
        path: PathBuf,
        cause: std::io::Error,
    },
    /// A step failed and undoing it failed as well; both errors are kept.
    Unrecovered {
        failure: Box<ExportTransactionError>,
        recovery: Box<ExportTransactionError>,
    },
}

#[derive(Debug)]
pub(crate) enum JournalDefect {
    /// Not a regular file, or larger than the journal bound.
    Unbounded,
    Malformed(serde_json::Error),
    /// Written for another schema or another artifact pair.
    WrongPair,
    InvalidGeneratedPath,
    InvalidSha256,
}

impl ExportTransactionError {
    fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Self + '_ {
        move |cause| Self::Io {
            path: path.to_path_buf(),
            cause,
        }
    }

    fn changed(path: &Path) -> Self {
        Self::TargetChanged {
            path: path.to_path_buf(),
        }
    }

    fn unrecovered(failure: Self, recovery: Self) -> Self {
        Self::Unrecovered {
            failure: Box::new(failure),
            recovery: Box::new(recovery),
        }
    }

    /// A no-clobber publish fails with `AlreadyExists` exactly when another writer created
    /// the target after it was checked.
    fn publish(path: &Path, cause: std::io::Error) -> Self {
        if cause.kind() == std::io::ErrorKind::AlreadyExists {
            Self::changed(path)
        } else {
            Self::io(path)(cause)
        }
    }
}

impl std::fmt::Display for JournalDefect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbounded => formatter.write_str("is not a bounded regular file"),
            Self::Malformed(error) => write!(formatter, "is malformed: {error}"),
            Self::WrongPair => formatter.write_str("does not match this artifact pair"),
            Self::InvalidGeneratedPath => formatter.write_str("contains an invalid generated path"),
            Self::InvalidSha256 => formatter.write_str("contains an invalid SHA-256"),
        }
    }
}

impl std::fmt::Display for ExportTransactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetChanged { path } => write!(
                formatter,
                "export target {} changed concurrently after authorization",
                path.display()
            ),
            Self::OriginalPreserved { path, preserved } => write!(
                formatter,
                "export target {} changed concurrently; authorized original preserved at {}",
                path.display(),
                preserved.display()
            ),
            Self::NotRegularFile { path } => {
                write!(formatter, "{} is not a regular file", path.display())
            }
            Self::PairNotSiblings => formatter.write_str(
                "export artifact and loss report must be distinct files in one directory",
            ),
            Self::UnnamedPath { path } => write!(
                formatter,
                "export path {} has no valid UTF-8 file name",
                path.display()
            ),
            Self::InvalidJournal(defect) => write!(formatter, "export recovery journal {defect}"),
            Self::BackupMissing { path } => write!(
                formatter,
                "export recovery backup is missing or invalid for {}",
                path.display()
            ),
            Self::Io { path, cause } => write!(formatter, "{}: {cause}", path.display()),
            Self::Unrecovered { failure, recovery } => {
                write!(formatter, "{failure}; export recovery failed: {recovery}")
            }
        }
    }
}

impl std::error::Error for ExportTransactionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { cause, .. } => Some(cause),
            Self::InvalidJournal(JournalDefect::Malformed(error)) => Some(error),
            Self::Unrecovered { recovery, .. } => Some(recovery.as_ref()),
            _ => None,
        }
    }
}

/// Why one export command was refused.
#[derive(Debug)]
pub(crate) enum ExportError {
    /// The chosen destination does not meet what this export writes, e.g. its extension.
    Destination { requirement: &'static str },
    /// The current document holds nothing of this kind to export.
    NothingToExport { subject: &'static str },
    /// More than one candidate of this kind exists where the export takes exactly one.
    Ambiguous { subject: &'static str },
    /// The visible content holds bodies this format cannot carry.
    UnsupportedContent { reason: &'static str },
    /// One visible occurrence cannot be exported; `reason` says what it lacks.
    OccurrenceNotExportable {
        occurrence: InstancePath,
        reason: &'static str,
    },
    /// The exported content changed while the human was asked for consent.
    ChangedDuringConsent,
    /// The produced artifact failed its read-back check.
    ArtifactUnverified { check: &'static str },
    /// Publishing the files was refused.
    Transaction(ExportTransactionError),
    /// A projection, exporter, worker or consent step refused; its error is the cause.
    Failed(BoxedCause),
}

impl ExportError {
    pub(crate) fn failed(cause: impl Into<BoxedCause>) -> Self {
        Self::Failed(cause.into())
    }
}

impl From<ExportTransactionError> for ExportError {
    fn from(error: ExportTransactionError) -> Self {
        Self::Transaction(error)
    }
}

/// A refused consent, worker or projection step is kept as the cause.
impl From<Rejection> for ExportError {
    fn from(error: Rejection) -> Self {
        Self::Failed(error.into())
    }
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Destination { requirement } => {
                write!(formatter, "this export requires {requirement}")
            }
            Self::NothingToExport { subject } => {
                write!(formatter, "the document contains no exportable {subject}")
            }
            Self::Ambiguous { subject } => {
                write!(formatter, "the export requires exactly one {subject}")
            }
            Self::UnsupportedContent { reason } => formatter.write_str(reason),
            Self::OccurrenceNotExportable { occurrence, reason } => {
                write!(formatter, "visible occurrence {occurrence:?} {reason}")
            }
            Self::ChangedDuringConsent => {
                formatter.write_str("the export changed while consent was pending")
            }
            Self::ArtifactUnverified { check } => {
                write!(formatter, "the exported artifact failed its check: {check}")
            }
            Self::Transaction(error) => error.fmt(formatter),
            Self::Failed(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transaction(error) => error.source(),
            Self::Failed(cause) => cause.source(),
            _ => None,
        }
    }
}

/// The consent one export bundle needs: its release or lossy-conversion confirmation and,
/// for each existing file it replaces, an overwrite confirmation.
pub(crate) struct ExportConsent<'a> {
    pub(crate) class: HighRiskClass,
    pub(crate) operation: &'a str,
    pub(crate) title_key: &'a str,
    pub(crate) risk_key: &'a str,
    pub(crate) overwrite_title_key: &'a str,
    /// Overwrite operations for the primary artifact and its companion report.
    pub(crate) overwrite_operations: [&'a str; 2],
}

impl KetchupApp {
    pub(crate) fn authorize_export_bundle(
        &mut self,
        consent: ExportConsent<'_>,
        primary_path: &Path,
        report_path: &Path,
        precondition: &ExportBundlePrecondition,
        evidence: &[u8],
    ) -> Result<(), ExportError> {
        let title = self.catalog.text(consent.title_key);
        let risk = self.catalog.text(consent.risk_key);
        self.authorize_path_side_effect(
            consent.class,
            consent.operation,
            &title,
            &risk,
            primary_path,
            evidence,
        )?;
        let replaced = [
            (primary_path, &precondition.primary_sha256),
            (report_path, &precondition.report_sha256),
        ];
        for ((path, existing), operation) in replaced.into_iter().zip(consent.overwrite_operations)
        {
            if existing.is_some() {
                let title = self.catalog.text(consent.overwrite_title_key);
                let risk = self.catalog.text("dialog-export-overwrite-risk");
                self.authorize_path_side_effect(
                    HighRiskClass::Overwrite,
                    operation,
                    &title,
                    &risk,
                    path,
                    evidence,
                )?;
            }
        }
        Ok(())
    }

    /// Reports one finished export under its catalog key (`digest-exported-<key>` or
    /// `error-export-<key>`); a refusal names its typed reason.
    pub(crate) fn report_export_outcome(
        &mut self,
        catalog_key: &str,
        path: &Path,
        result: Result<(), ExportError>,
    ) -> bool {
        match result {
            Ok(()) => {
                self.digest = self.catalog.format(
                    &format!("digest-exported-{catalog_key}"),
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    &format!("error-export-{catalog_key}"),
                    &BTreeMap::from([
                        ("path", path.display().to_string()),
                        ("reason", error.to_string()),
                    ]),
                );
                false
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportBundleJournal {
    schema: String,
    primary_path_sha256: String,
    report_path_sha256: String,
    primary_temporary_name: String,
    report_temporary_name: String,
    primary_backup_name: Option<String>,
    report_backup_name: Option<String>,
    original_primary_sha256: Option<String>,
    original_report_sha256: Option<String>,
    published_primary_sha256: String,
    published_report_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExportBundlePrecondition {
    pub(crate) primary_sha256: Option<String>,
    pub(crate) report_sha256: Option<String>,
}

impl ExportBundlePrecondition {
    pub(crate) fn capture(
        primary_path: &Path,
        report_path: &Path,
    ) -> Result<Self, ExportTransactionError> {
        recover_export_bundle(primary_path, report_path)?;
        Ok(Self {
            primary_sha256: export_target_sha256(primary_path)?,
            report_sha256: export_target_sha256(report_path)?,
        })
    }
}

pub(crate) fn export_target_sha256(path: &Path) -> Result<Option<String>, ExportTransactionError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ExportTransactionError::io(path)(error)),
    };
    if !file
        .metadata()
        .map_err(ExportTransactionError::io(path))?
        .is_file()
    {
        return Err(ExportTransactionError::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    ketchup_model::graph::sha256_reader_hex(file)
        .map(Some)
        .map_err(ExportTransactionError::io(path))
}

fn export_path_identity_sha256(path: &Path) -> String {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt as _;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(windows)]
    let bytes = {
        use std::os::windows::ffi::OsStrExt as _;
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    };
    #[cfg(not(any(unix, windows)))]
    let bytes = path.to_string_lossy().as_bytes().to_vec();
    ketchup_model::graph::sha256_hex(&bytes)
}

fn export_bundle_journal_path(primary_path: &Path) -> Result<PathBuf, ExportTransactionError> {
    let mut name = primary_path
        .file_name()
        .ok_or_else(|| ExportTransactionError::UnnamedPath {
            path: primary_path.to_path_buf(),
        })?
        .to_os_string();
    name.push(".ketchup-export-journal");
    Ok(primary_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(name))
}

fn export_generated_name(path: &Path) -> Result<String, ExportTransactionError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| ExportTransactionError::UnnamedPath {
            path: path.to_path_buf(),
        })
}

fn resolve_export_generated_path(
    parent: &Path,
    name: &str,
    prefix: &str,
) -> Result<PathBuf, ExportTransactionError> {
    let mut components = Path::new(name).components();
    if !name.starts_with(prefix)
        || !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(ExportTransactionError::InvalidJournal(
            JournalDefect::InvalidGeneratedPath,
        ));
    }
    Ok(parent.join(name))
}

fn valid_export_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sync_export_parent(parent: &Path) -> Result<(), ExportTransactionError> {
    #[cfg(unix)]
    {
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(ExportTransactionError::io(parent))
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}

fn export_artifact_lock_path(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".ketchup-export-lock");
    PathBuf::from(lock_path)
}

fn export_parent(path: &Path) -> &Path {
    path.parent().unwrap_or_else(|| Path::new("."))
}

pub(crate) fn write_export_artifact_if_unchanged(
    path: &Path,
    bytes: &[u8],
    expected_sha256: Option<&str>,
) -> Result<(), ExportTransactionError> {
    write_export_artifact_if_unchanged_after_compare(path, bytes, expected_sha256, || {})
}

fn write_export_artifact_if_unchanged_after_compare(
    path: &Path,
    bytes: &[u8],
    expected_sha256: Option<&str>,
    after_compare: impl FnOnce(),
) -> Result<(), ExportTransactionError> {
    if path.is_dir() {
        return Err(ExportTransactionError::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    let parent = export_parent(path);
    let lock_path = export_artifact_lock_path(path);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(ExportTransactionError::io(&lock_path))?;
    lock.lock()
        .map_err(ExportTransactionError::io(&lock_path))?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(ExportTransactionError::io(parent))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(ExportTransactionError::io(temporary.path()))?;
    if export_target_sha256(path)?.as_deref() != expected_sha256 {
        return Err(ExportTransactionError::changed(path));
    }
    let backup = expected_sha256
        .map(|_| empty_export_temp_path(path, ".ketchup-export-backup-"))
        .transpose()?;
    move_export_target_to_backup(path, expected_sha256, backup.as_deref())?;
    after_compare();
    match temporary.persist_noclobber(path) {
        Ok(_) => {
            if let Some(backup) = backup {
                let backup_path = backup.to_path_buf();
                backup
                    .close()
                    .map_err(ExportTransactionError::io(&backup_path))?;
            }
            sync_export_parent(parent)
        }
        Err(error) => {
            let publish_error = ExportTransactionError::publish(path, error.error);
            match backup {
                Some(backup) if !path.exists() => {
                    if let Err(error) = std::fs::rename(&backup, path) {
                        return Err(ExportTransactionError::unrecovered(
                            publish_error,
                            ExportTransactionError::io(&backup)(error),
                        ));
                    }
                    sync_export_parent(parent)?;
                    Err(publish_error)
                }
                Some(backup) => {
                    let backup_path = backup.to_path_buf();
                    let preserved = backup.keep().map_err(|error| {
                        ExportTransactionError::unrecovered(
                            publish_error,
                            ExportTransactionError::io(&backup_path)(error.error),
                        )
                    })?;
                    Err(ExportTransactionError::OriginalPreserved {
                        path: path.to_path_buf(),
                        preserved,
                    })
                }
                None => Err(publish_error),
            }
        }
    }
}

fn empty_export_temp_path(
    path: &Path,
    prefix: &str,
) -> Result<tempfile::TempPath, ExportTransactionError> {
    let parent = export_parent(path);
    let temporary = tempfile::Builder::new()
        .prefix(prefix)
        .tempfile_in(parent)
        .map_err(ExportTransactionError::io(parent))?
        .into_temp_path();
    std::fs::remove_file(&temporary).map_err(ExportTransactionError::io(&temporary))?;
    Ok(temporary)
}

fn move_export_target_to_backup(
    path: &Path,
    expected_sha256: Option<&str>,
    backup_path: Option<&Path>,
) -> Result<(), ExportTransactionError> {
    move_export_target_to_backup_after_compare(path, expected_sha256, backup_path, || {})
}

fn move_export_target_to_backup_after_compare(
    path: &Path,
    expected_sha256: Option<&str>,
    backup_path: Option<&Path>,
    after_compare: impl FnOnce(),
) -> Result<(), ExportTransactionError> {
    if export_target_sha256(path)?.as_deref() != expected_sha256 {
        return Err(ExportTransactionError::changed(path));
    }
    after_compare();
    match (expected_sha256, backup_path) {
        (None, None) => Ok(()),
        (Some(expected_sha256), Some(backup_path)) => {
            std::fs::rename(path, backup_path).map_err(ExportTransactionError::io(path))?;
            let moved_sha256 = export_target_sha256(backup_path);
            if !matches!(&moved_sha256, Ok(Some(actual)) if actual == expected_sha256) {
                let failure = match moved_sha256 {
                    Ok(_) => ExportTransactionError::changed(path),
                    Err(error) => error,
                };
                std::fs::hard_link(backup_path, path).map_err(|error| {
                    ExportTransactionError::unrecovered(
                        ExportTransactionError::changed(path),
                        ExportTransactionError::io(path)(error),
                    )
                })?;
                std::fs::remove_file(backup_path)
                    .map_err(ExportTransactionError::io(backup_path))?;
                sync_export_parent(export_parent(path))?;
                return Err(failure);
            }
            sync_export_parent(export_parent(path))
        }
        _ => Err(ExportTransactionError::BackupMissing {
            path: path.to_path_buf(),
        }),
    }
}

fn remove_export_artifact(
    path: &Path,
    expected_sha256: &str,
) -> Result<(), ExportTransactionError> {
    match export_target_sha256(path)? {
        None => Ok(()),
        Some(actual) if actual == expected_sha256 => {
            std::fs::remove_file(path).map_err(ExportTransactionError::io(path))
        }
        Some(_) => Err(ExportTransactionError::changed(path)),
    }
}

fn validate_export_target_recovery(
    path: &Path,
    backup_path: Option<&Path>,
    original_sha256: Option<&str>,
    published_sha256: &str,
) -> Result<(), ExportTransactionError> {
    let current_sha256 = export_target_sha256(path)?;
    if current_sha256.as_deref() == original_sha256
        || current_sha256.as_deref() == Some(published_sha256)
    {
        return Ok(());
    }
    if current_sha256.is_none() && original_sha256.is_some() {
        let backup_path = backup_path.ok_or_else(|| ExportTransactionError::BackupMissing {
            path: path.to_path_buf(),
        })?;
        if export_target_sha256(backup_path)?.as_deref() == original_sha256 {
            return Ok(());
        }
    }
    Err(ExportTransactionError::changed(path))
}

fn recover_export_target(
    path: &Path,
    backup_path: Option<&Path>,
    original_sha256: Option<&str>,
    published_sha256: &str,
) -> Result<(), ExportTransactionError> {
    let current_sha256 = export_target_sha256(path)?;
    match original_sha256 {
        None => match current_sha256.as_deref() {
            None => Ok(()),
            Some(current) if current == published_sha256 => {
                std::fs::remove_file(path).map_err(ExportTransactionError::io(path))
            }
            Some(_) => Err(ExportTransactionError::changed(path)),
        },
        Some(original_sha256) => {
            if current_sha256.as_deref() == Some(original_sha256) {
                return Ok(());
            }
            if current_sha256.is_some() && current_sha256.as_deref() != Some(published_sha256) {
                return Err(ExportTransactionError::changed(path));
            }
            let missing = || ExportTransactionError::BackupMissing {
                path: path.to_path_buf(),
            };
            let backup_path = backup_path.ok_or_else(missing)?;
            if export_target_sha256(backup_path)?.as_deref() != Some(original_sha256) {
                return Err(missing());
            }
            if current_sha256.is_some() {
                std::fs::remove_file(path).map_err(ExportTransactionError::io(path))?;
            }
            std::fs::rename(backup_path, path).map_err(ExportTransactionError::io(backup_path))
        }
    }
}

fn read_export_bundle_journal(path: &Path) -> Result<Option<Vec<u8>>, ExportTransactionError> {
    let unbounded = || ExportTransactionError::InvalidJournal(JournalDefect::Unbounded);
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ExportTransactionError::io(path)(error)),
    };
    if !file
        .metadata()
        .map_err(ExportTransactionError::io(path))?
        .is_file()
    {
        return Err(unbounded());
    }
    let mut bytes = Vec::new();
    file.take(MAX_EXPORT_BUNDLE_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ExportTransactionError::io(path))?;
    if bytes.len() as u64 > MAX_EXPORT_BUNDLE_JOURNAL_BYTES {
        return Err(unbounded());
    }
    Ok(Some(bytes))
}

pub(crate) fn recover_export_bundle(
    primary_path: &Path,
    report_path: &Path,
) -> Result<(), ExportTransactionError> {
    let journal_path = export_bundle_journal_path(primary_path)?;
    let Some(journal_bytes) = read_export_bundle_journal(&journal_path)? else {
        return Ok(());
    };
    let journal: ExportBundleJournal = serde_json::from_slice(&journal_bytes)
        .map_err(|error| ExportTransactionError::InvalidJournal(JournalDefect::Malformed(error)))?;
    if journal.schema != EXPORT_BUNDLE_JOURNAL_SCHEMA_V1
        || journal.primary_path_sha256 != export_path_identity_sha256(primary_path)
        || journal.report_path_sha256 != export_path_identity_sha256(report_path)
    {
        return Err(ExportTransactionError::InvalidJournal(
            JournalDefect::WrongPair,
        ));
    }
    let hashes = [
        journal.original_primary_sha256.as_deref(),
        journal.original_report_sha256.as_deref(),
        Some(journal.published_primary_sha256.as_str()),
        Some(journal.published_report_sha256.as_str()),
    ];
    if hashes
        .into_iter()
        .flatten()
        .any(|hash| !valid_export_sha256(hash))
    {
        return Err(ExportTransactionError::InvalidJournal(
            JournalDefect::InvalidSha256,
        ));
    }
    let parent = export_parent(primary_path);
    if parent != export_parent(report_path) {
        return Err(ExportTransactionError::PairNotSiblings);
    }
    let primary_temporary = resolve_export_generated_path(
        parent,
        &journal.primary_temporary_name,
        ".ketchup-export-primary-",
    )?;
    let report_temporary = resolve_export_generated_path(
        parent,
        &journal.report_temporary_name,
        ".ketchup-export-report-",
    )?;
    let primary_backup = journal
        .primary_backup_name
        .as_deref()
        .map(|name| resolve_export_generated_path(parent, name, ".ketchup-export-backup-"))
        .transpose()?;
    let report_backup = journal
        .report_backup_name
        .as_deref()
        .map(|name| resolve_export_generated_path(parent, name, ".ketchup-export-backup-"))
        .transpose()?;

    let fully_published = export_target_sha256(primary_path)?.as_deref()
        == Some(journal.published_primary_sha256.as_str())
        && export_target_sha256(report_path)?.as_deref()
            == Some(journal.published_report_sha256.as_str());
    if !fully_published {
        validate_export_target_recovery(
            primary_path,
            primary_backup.as_deref(),
            journal.original_primary_sha256.as_deref(),
            &journal.published_primary_sha256,
        )?;
        validate_export_target_recovery(
            report_path,
            report_backup.as_deref(),
            journal.original_report_sha256.as_deref(),
            &journal.published_report_sha256,
        )?;
        recover_export_target(
            primary_path,
            primary_backup.as_deref(),
            journal.original_primary_sha256.as_deref(),
            &journal.published_primary_sha256,
        )?;
        recover_export_target(
            report_path,
            report_backup.as_deref(),
            journal.original_report_sha256.as_deref(),
            &journal.published_report_sha256,
        )?;
    }
    for (path, expected_sha256) in [
        (
            primary_temporary.as_path(),
            &journal.published_primary_sha256,
        ),
        (report_temporary.as_path(), &journal.published_report_sha256),
    ] {
        remove_export_artifact(path, expected_sha256)?;
    }
    for (path, expected_sha256) in [
        (
            primary_backup.as_deref(),
            journal.original_primary_sha256.as_deref(),
        ),
        (
            report_backup.as_deref(),
            journal.original_report_sha256.as_deref(),
        ),
    ] {
        if let (Some(path), Some(expected_sha256)) = (path, expected_sha256) {
            remove_export_artifact(path, expected_sha256)?;
        }
    }
    sync_export_parent(parent)?;
    std::fs::remove_file(&journal_path).map_err(ExportTransactionError::io(&journal_path))?;
    sync_export_parent(parent)
}

fn persist_export_journal(
    primary_path: &Path,
    journal: &ExportBundleJournal,
) -> Result<(), ExportTransactionError> {
    let journal_path = export_bundle_journal_path(primary_path)?;
    let parent = export_parent(primary_path);
    let bytes = serde_json::to_vec(journal)
        .map_err(|error| ExportTransactionError::InvalidJournal(JournalDefect::Malformed(error)))?;
    if bytes.len() as u64 > MAX_EXPORT_BUNDLE_JOURNAL_BYTES {
        return Err(ExportTransactionError::InvalidJournal(
            JournalDefect::Unbounded,
        ));
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".ketchup-export-journal-")
        .tempfile_in(parent)
        .map_err(ExportTransactionError::io(parent))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(ExportTransactionError::io(temporary.path()))?;
    temporary
        .into_temp_path()
        .persist_noclobber(&journal_path)
        .map_err(|error| ExportTransactionError::publish(&journal_path, error.error))?;
    sync_export_parent(parent)
}

fn persist_export_temporary(
    temporary: tempfile::TempPath,
    path: &Path,
) -> Result<(), ExportTransactionError> {
    temporary
        .persist_noclobber(path)
        .map(|_| ())
        .map_err(|error| ExportTransactionError::publish(path, error.error))
}

fn sealed_temporary(
    parent: &Path,
    prefix: &str,
    bytes: &[u8],
) -> Result<tempfile::TempPath, ExportTransactionError> {
    let mut temporary = tempfile::Builder::new()
        .prefix(prefix)
        .tempfile_in(parent)
        .map_err(ExportTransactionError::io(parent))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(ExportTransactionError::io(temporary.path()))?;
    Ok(temporary.into_temp_path())
}

pub(crate) fn write_export_bundle(
    primary_path: &Path,
    primary: &[u8],
    report_path: &Path,
    report: &[u8],
    precondition: &ExportBundlePrecondition,
) -> Result<(), ExportTransactionError> {
    let primary_parent = export_parent(primary_path);
    if primary_parent != export_parent(report_path) || primary_path == report_path {
        return Err(ExportTransactionError::PairNotSiblings);
    }
    for path in [primary_path, report_path] {
        if path.is_dir() {
            return Err(ExportTransactionError::NotRegularFile {
                path: path.to_path_buf(),
            });
        }
    }
    recover_export_bundle(primary_path, report_path)?;
    let primary_temporary = sealed_temporary(primary_parent, ".ketchup-export-primary-", primary)?;
    let report_temporary = sealed_temporary(primary_parent, ".ketchup-export-report-", report)?;
    let primary_backup = precondition
        .primary_sha256
        .as_ref()
        .map(|_| empty_export_temp_path(primary_path, ".ketchup-export-backup-"))
        .transpose()?;
    let report_backup = precondition
        .report_sha256
        .as_ref()
        .map(|_| empty_export_temp_path(report_path, ".ketchup-export-backup-"))
        .transpose()?;
    let journal = ExportBundleJournal {
        schema: EXPORT_BUNDLE_JOURNAL_SCHEMA_V1.to_owned(),
        primary_path_sha256: export_path_identity_sha256(primary_path),
        report_path_sha256: export_path_identity_sha256(report_path),
        primary_temporary_name: export_generated_name(&primary_temporary)?,
        report_temporary_name: export_generated_name(&report_temporary)?,
        primary_backup_name: primary_backup
            .as_deref()
            .map(export_generated_name)
            .transpose()?,
        report_backup_name: report_backup
            .as_deref()
            .map(export_generated_name)
            .transpose()?,
        original_primary_sha256: precondition.primary_sha256.clone(),
        original_report_sha256: precondition.report_sha256.clone(),
        published_primary_sha256: ketchup_model::graph::sha256_hex(primary),
        published_report_sha256: ketchup_model::graph::sha256_hex(report),
    };
    persist_export_journal(primary_path, &journal)?;

    let publish = (|| {
        move_export_target_to_backup(
            primary_path,
            precondition.primary_sha256.as_deref(),
            primary_backup.as_deref(),
        )?;
        move_export_target_to_backup(
            report_path,
            precondition.report_sha256.as_deref(),
            report_backup.as_deref(),
        )?;
        persist_export_temporary(primary_temporary, primary_path)?;
        sync_export_parent(primary_parent)?;
        persist_export_temporary(report_temporary, report_path)?;
        sync_export_parent(primary_parent)
    })();
    if let Err(error) = publish {
        return Err(match recover_export_bundle(primary_path, report_path) {
            Ok(()) => error,
            Err(recovery) => ExportTransactionError::unrecovered(error, recovery),
        });
    }
    recover_export_bundle(primary_path, report_path)
}

fn exact_mesh_export_evidence(bundle: &ExactMeshExport) -> Vec<u8> {
    let mut evidence = b"ketchup.exact-mesh-export.v1".to_vec();
    for artifact in [&bundle.mesh_obj, &bundle.loss_report] {
        evidence.extend_from_slice(&(artifact.len() as u64).to_le_bytes());
        evidence.extend_from_slice(artifact.as_bytes());
    }
    evidence
}

fn write_exact_mesh_export(
    path: &Path,
    bundle: ExactMeshExport,
) -> Result<(), ExportTransactionError> {
    let report_path = path.with_extension("obj.loss.txt");
    let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
    write_export_bundle(
        path,
        bundle.mesh_obj.as_bytes(),
        &report_path,
        bundle.loss_report.as_bytes(),
        &precondition,
    )
}

impl KetchupApp {
    pub fn export_exact_occurrence_mesh_to(
        &mut self,
        instance_path: &InstancePath,
        path: &Path,
    ) -> bool {
        let snapshot = self.document.current();
        let result = (|| {
            let occurrence = snapshot
                .scene_query()
                .into_iter()
                .find(|occurrence| &occurrence.instance_path == instance_path)
                .ok_or(ExportError::NothingToExport {
                    subject: "canonical occurrence at this instance path",
                })?;
            let bundle = self
                .exact
                .results
                .get_render(&snapshot, occurrence.definition_id)
                .ok_or(ExportError::NothingToExport {
                    subject: "unique current visible exact body result",
                })?
                .mesh_export(occurrence.transform);
            self.authorize_path_side_effect(
                HighRiskClass::LossyConversion,
                "export-lossy-obj-with-loss-report",
                "Confirm lossy mesh export",
                "lossy exact-to-mesh conversion",
                path,
                &exact_mesh_export_evidence(&bundle),
            )?;
            Ok::<_, ExportError>(write_exact_mesh_export(path, bundle)?)
        })();
        match result {
            Ok(()) => {
                self.digest = format!(
                    "Exported transformed exact occurrence OBJ with explicit loss report to {}",
                    path.display()
                );
                true
            }
            Err(error) => {
                self.digest = format!("Exact occurrence mesh export blocked: {error}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_refusals_name_their_typed_reason_and_keep_the_cause() {
        let directory = tempfile::tempdir().unwrap();
        let primary = directory.path().join("model.step");
        let report = directory.path().join("model.step.loss.txt");

        let error = KetchupApp::validate_homag_mpr_path(Path::new("123456789012.nc")).unwrap_err();
        assert!(matches!(
            error,
            ExportError::Destination { requirement } if requirement.contains(".mpr")
        ));
        assert!(matches!(
            KetchupApp::validate_homag_mpr_path(Path::new("short.mpr")).unwrap_err(),
            ExportError::Destination { .. }
        ));

        let precondition = ExportBundlePrecondition::capture(&primary, &report).unwrap();
        let error =
            write_export_bundle(&primary, b"cad", &primary, b"report", &precondition).unwrap_err();
        assert!(matches!(error, ExportTransactionError::PairNotSiblings));

        std::fs::write(export_bundle_journal_path(&primary).unwrap(), b"{").unwrap();
        let error = ExportError::from(recover_export_bundle(&primary, &report).unwrap_err());
        let ExportError::Transaction(ExportTransactionError::InvalidJournal(
            JournalDefect::Malformed(_),
        )) = &error
        else {
            panic!("a malformed journal must be named, got {error:?}");
        };
        assert!(
            error
                .to_string()
                .starts_with("export recovery journal is malformed")
        );

        let error = ExportError::from(ExportTransactionError::Io {
            path: primary.clone(),
            cause: std::io::Error::other("disk gone"),
        });
        let cause = std::error::Error::source(&error).expect("the I/O error stays the cause");
        assert!(cause.downcast_ref::<std::io::Error>().is_some());

        let mut app = KetchupApp::new();
        assert!(!app.export_current_profiles_dxf_to(&directory.path().join("plan.dwg")));
        assert!(
            app.digest
                .contains("an explicit .dxf destination; native DWG export is unavailable"),
            "{}",
            app.digest
        );
    }

    #[test]
    fn export_target_sha256_streams_large_existing_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("large.step");
        let mut file = std::fs::File::create(&target).unwrap();
        for _ in 0..1_000 {
            std::io::Write::write_all(&mut file, &[b'a'; 1_000]).unwrap();
        }
        drop(file);

        assert_eq!(
            export_target_sha256(&target).unwrap().as_deref(),
            Some("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0")
        );
    }

    #[test]
    fn export_recovery_journal_rejects_actual_bytes_above_limit() {
        let directory = tempfile::tempdir().unwrap();
        let primary = directory.path().join("model.step");
        let report = directory.path().join("model.step.loss.txt");
        let journal = export_bundle_journal_path(&primary).unwrap();
        let mut file = std::fs::File::create(journal).unwrap();
        file.set_len(MAX_EXPORT_BUNDLE_JOURNAL_BYTES + 1).unwrap();
        std::io::Write::write_all(&mut file, b"{").unwrap();
        drop(file);

        let error = recover_export_bundle(&primary, &report).unwrap_err();
        assert!(matches!(
            error,
            ExportTransactionError::InvalidJournal(JournalDefect::Unbounded)
        ));
        assert_eq!(
            error.to_string(),
            "export recovery journal is not a bounded regular file"
        );
    }

    #[test]
    fn export_single_artifact_preserves_replacement_after_precondition_check() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("drawing.svg");
        std::fs::write(&target, b"authorized original").unwrap();
        let original_sha256 = export_target_sha256(&target).unwrap().unwrap();

        let error = write_export_artifact_if_unchanged_after_compare(
            &target,
            b"ketchup export",
            Some(&original_sha256),
            || {
                let replacement = directory.path().join("replacement.tmp");
                std::fs::write(&replacement, b"external replacement").unwrap();
                std::fs::rename(replacement, &target).unwrap();
            },
        )
        .unwrap_err();

        let ExportTransactionError::OriginalPreserved { path, preserved } = &error else {
            panic!("a replaced target must keep the authorized original, got {error:?}");
        };
        assert_eq!(path, &target);
        assert!(error.to_string().contains("changed concurrently"));
        assert_eq!(std::fs::read(preserved).unwrap(), b"authorized original");
        assert_eq!(std::fs::read(&target).unwrap(), b"external replacement");
        let preserved = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".ketchup-export-backup-")
            })
            .unwrap();
        assert_eq!(std::fs::read(preserved).unwrap(), b"authorized original");
    }

    #[test]
    fn export_backup_move_preserves_replacement_after_precondition_check() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("model.step");
        std::fs::write(&target, b"authorized original").unwrap();
        let original_sha256 = export_target_sha256(&target).unwrap().unwrap();
        let backup = empty_export_temp_path(&target, ".ketchup-export-backup-").unwrap();
        let backup_path = backup.to_path_buf();

        let error = move_export_target_to_backup_after_compare(
            &target,
            Some(&original_sha256),
            Some(&backup_path),
            || {
                let replacement = directory.path().join("replacement.tmp");
                std::fs::write(&replacement, b"external replacement").unwrap();
                std::fs::rename(replacement, &target).unwrap();
            },
        )
        .unwrap_err();

        assert!(matches!(
            &error,
            ExportTransactionError::TargetChanged { path } if path == &target
        ));
        assert_eq!(std::fs::read(&target).unwrap(), b"external replacement");
        assert!(!backup_path.exists());
    }

    #[test]
    fn export_rollback_preserves_concurrent_destination_and_original_backup() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("model.step");
        std::fs::write(&target, b"original").unwrap();
        let original_sha256 = export_target_sha256(&target).unwrap().unwrap();
        let backup = empty_export_temp_path(&target, ".ketchup-export-backup-").unwrap();
        let backup_path = backup.to_path_buf();
        move_export_target_to_backup(&target, Some(&original_sha256), Some(&backup_path)).unwrap();
        std::fs::write(&target, b"concurrent writer").unwrap();

        let error = recover_export_target(
            &target,
            Some(&backup_path),
            Some(&original_sha256),
            &ketchup_model::graph::sha256_hex(b"intended export"),
        )
        .unwrap_err();

        assert!(matches!(
            &error,
            ExportTransactionError::TargetChanged { path } if path == &target
        ));
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent writer");
        assert_eq!(std::fs::read(backup_path).unwrap(), b"original");
    }

    #[test]
    fn export_rollback_replaces_only_its_own_published_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("model.step");
        std::fs::write(&target, b"original").unwrap();
        let original_sha256 = export_target_sha256(&target).unwrap().unwrap();
        let backup = empty_export_temp_path(&target, ".ketchup-export-backup-").unwrap();
        let backup_path = backup.to_path_buf();
        move_export_target_to_backup(&target, Some(&original_sha256), Some(&backup_path)).unwrap();
        let published = b"ketchup export";
        std::fs::write(&target, published).unwrap();
        let published_sha256 = ketchup_model::graph::sha256_hex(published);

        recover_export_target(
            &target,
            Some(&backup_path),
            Some(&original_sha256),
            &published_sha256,
        )
        .unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert!(!backup_path.exists());
    }

    #[test]
    fn exact_mesh_export_failure_does_not_partially_replace_existing_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let mesh_path = directory.path().join("protected.obj");
        let report_path = mesh_path.with_extension("obj.loss.txt");
        std::fs::write(&mesh_path, b"existing mesh").unwrap();
        std::fs::create_dir(&report_path).unwrap();

        let error = write_exact_mesh_export(
            &mesh_path,
            ExactMeshExport {
                mesh_obj: "replacement mesh".to_owned(),
                loss_report: "replacement report".to_owned(),
            },
        )
        .unwrap_err();

        // Unix opens the directory and finds it is not a file; Windows refuses to open it.
        assert!(matches!(
            &error,
            ExportTransactionError::NotRegularFile { path }
                | ExportTransactionError::Io { path, .. } if path == &report_path
        ));
        assert_eq!(std::fs::read(&mesh_path).unwrap(), b"existing mesh");
        assert!(report_path.is_dir());
    }

    fn staged_export_restart_journal(
        primary_path: &Path,
        report_path: &Path,
        original_primary: &[u8],
        original_report: &[u8],
        published_primary: &[u8],
        published_report: &[u8],
    ) -> ExportBundleJournal {
        ExportBundleJournal {
            schema: EXPORT_BUNDLE_JOURNAL_SCHEMA_V1.to_owned(),
            primary_path_sha256: export_path_identity_sha256(primary_path),
            report_path_sha256: export_path_identity_sha256(report_path),
            primary_temporary_name: ".ketchup-export-primary-restart".to_owned(),
            report_temporary_name: ".ketchup-export-report-restart".to_owned(),
            primary_backup_name: Some(".ketchup-export-backup-primary-restart".to_owned()),
            report_backup_name: Some(".ketchup-export-backup-report-restart".to_owned()),
            original_primary_sha256: Some(ketchup_model::graph::sha256_hex(original_primary)),
            original_report_sha256: Some(ketchup_model::graph::sha256_hex(original_report)),
            published_primary_sha256: ketchup_model::graph::sha256_hex(published_primary),
            published_report_sha256: ketchup_model::graph::sha256_hex(published_report),
        }
    }

    #[test]
    fn export_bundle_restart_rolls_back_a_partially_published_pair() {
        let directory = tempfile::tempdir().unwrap();
        let primary_path = directory.path().join("model.step");
        let report_path = directory.path().join("model.step.loss.txt");
        let original_primary = b"original CAD";
        let original_report = b"original loss report";
        let published_primary = b"new CAD";
        let published_report = b"new loss report";
        std::fs::write(&primary_path, original_primary).unwrap();
        std::fs::write(&report_path, original_report).unwrap();
        let journal = staged_export_restart_journal(
            &primary_path,
            &report_path,
            original_primary,
            original_report,
            published_primary,
            published_report,
        );
        let primary_temporary = directory.path().join(&journal.primary_temporary_name);
        let report_temporary = directory.path().join(&journal.report_temporary_name);
        let primary_backup = directory
            .path()
            .join(journal.primary_backup_name.as_deref().unwrap());
        let report_backup = directory
            .path()
            .join(journal.report_backup_name.as_deref().unwrap());
        std::fs::write(&primary_temporary, published_primary).unwrap();
        std::fs::write(&report_temporary, published_report).unwrap();
        persist_export_journal(&primary_path, &journal).unwrap();
        std::fs::rename(&primary_path, &primary_backup).unwrap();
        std::fs::rename(&report_path, &report_backup).unwrap();
        std::fs::rename(&primary_temporary, &primary_path).unwrap();

        recover_export_bundle(&primary_path, &report_path).unwrap();

        assert_eq!(std::fs::read(&primary_path).unwrap(), original_primary);
        assert_eq!(std::fs::read(&report_path).unwrap(), original_report);
        for path in [
            export_bundle_journal_path(&primary_path).unwrap(),
            primary_temporary,
            report_temporary,
            primary_backup,
            report_backup,
        ] {
            assert!(!path.exists(), "recovery residue: {}", path.display());
        }
    }

    #[test]
    fn export_bundle_restart_commits_a_fully_published_pair() {
        let directory = tempfile::tempdir().unwrap();
        let primary_path = directory.path().join("model.iges");
        let report_path = directory.path().join("model.iges.loss.txt");
        let original_primary = b"original CAD";
        let original_report = b"original loss report";
        let published_primary = b"new CAD";
        let published_report = b"new loss report";
        std::fs::write(&primary_path, original_primary).unwrap();
        std::fs::write(&report_path, original_report).unwrap();
        let journal = staged_export_restart_journal(
            &primary_path,
            &report_path,
            original_primary,
            original_report,
            published_primary,
            published_report,
        );
        let primary_temporary = directory.path().join(&journal.primary_temporary_name);
        let report_temporary = directory.path().join(&journal.report_temporary_name);
        let primary_backup = directory
            .path()
            .join(journal.primary_backup_name.as_deref().unwrap());
        let report_backup = directory
            .path()
            .join(journal.report_backup_name.as_deref().unwrap());
        std::fs::write(&primary_temporary, published_primary).unwrap();
        std::fs::write(&report_temporary, published_report).unwrap();
        persist_export_journal(&primary_path, &journal).unwrap();
        std::fs::rename(&primary_path, &primary_backup).unwrap();
        std::fs::rename(&report_path, &report_backup).unwrap();
        std::fs::rename(&primary_temporary, &primary_path).unwrap();
        std::fs::rename(&report_temporary, &report_path).unwrap();

        recover_export_bundle(&primary_path, &report_path).unwrap();

        assert_eq!(std::fs::read(&primary_path).unwrap(), published_primary);
        assert_eq!(std::fs::read(&report_path).unwrap(), published_report);
        for path in [
            export_bundle_journal_path(&primary_path).unwrap(),
            primary_temporary,
            report_temporary,
            primary_backup,
            report_backup,
        ] {
            assert!(!path.exists(), "recovery residue: {}", path.display());
        }
    }

    #[test]
    fn export_bundle_restart_refuses_concurrent_change_before_recovery_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let primary_path = directory.path().join("model.dxf");
        let report_path = directory.path().join("model.dxf.loss.txt");
        let original_primary = b"original CAD";
        let original_report = b"original loss report";
        let published_primary = b"new CAD";
        let published_report = b"new loss report";
        std::fs::write(&primary_path, original_primary).unwrap();
        std::fs::write(&report_path, original_report).unwrap();
        let journal = staged_export_restart_journal(
            &primary_path,
            &report_path,
            original_primary,
            original_report,
            published_primary,
            published_report,
        );
        let primary_temporary = directory.path().join(&journal.primary_temporary_name);
        let report_temporary = directory.path().join(&journal.report_temporary_name);
        let primary_backup = directory
            .path()
            .join(journal.primary_backup_name.as_deref().unwrap());
        let report_backup = directory
            .path()
            .join(journal.report_backup_name.as_deref().unwrap());
        std::fs::write(&primary_temporary, published_primary).unwrap();
        std::fs::write(&report_temporary, published_report).unwrap();
        persist_export_journal(&primary_path, &journal).unwrap();
        std::fs::rename(&primary_path, &primary_backup).unwrap();
        std::fs::rename(&report_path, &report_backup).unwrap();
        std::fs::rename(&primary_temporary, &primary_path).unwrap();
        std::fs::write(&report_path, b"concurrent writer").unwrap();

        let error = recover_export_bundle(&primary_path, &report_path).unwrap_err();

        assert!(matches!(
            &error,
            ExportTransactionError::TargetChanged { path } if path == &report_path
        ));
        assert_eq!(std::fs::read(&primary_path).unwrap(), published_primary);
        assert_eq!(std::fs::read(&report_path).unwrap(), b"concurrent writer");
        assert_eq!(std::fs::read(&primary_backup).unwrap(), original_primary);
        assert_eq!(std::fs::read(&report_backup).unwrap(), original_report);
        assert!(export_bundle_journal_path(&primary_path).unwrap().exists());
    }
}
