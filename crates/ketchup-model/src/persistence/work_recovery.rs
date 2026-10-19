use sha2::{Digest, Sha256};

use super::*;

/// Exclusive ownership of a live session's recovery branch. The OS releases the
/// lock on process exit; the lock file itself must never be unlinked.
#[derive(Debug)]
pub struct WorkRecoveryLock {
    _file: fs::File,
}

impl WorkRecoveryLock {
    pub fn acquire(path: &Path) -> Result<Self, FilePersistenceError> {
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".work-recovery-lock");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(FilePersistenceError::ExternalConflict),
            Err(std::fs::TryLockError::Error(error)) => Err(FilePersistenceError::Io(error)),
        }
    }
}

/// Checks the observed checkpoint while the caller holds its WorkRecoveryLock.
pub fn check_work_recovery_identity(
    path: &Path,
    expected: Option<FileIdentity>,
) -> Result<(), FilePersistenceError> {
    read_observed_work_recovery(path, expected).map(drop)
}

/// The checkpoint [`check_work_recovery_identity`] found as `expected`, with the
/// SHA-256 state of its bytes so a step appended to it is hashed alone.
struct ObservedWorkRecovery {
    bytes: Vec<u8>,
    hasher: Sha256,
}

fn read_observed_work_recovery(
    path: &Path,
    expected: Option<FileIdentity>,
) -> Result<Option<ObservedWorkRecovery>, FilePersistenceError> {
    let path = work_recovery_path(path);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound && expected.is_none() => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(FilePersistenceError::ExternalConflict)
        }
        Err(error) => Err(FilePersistenceError::Io(error)),
        Ok(metadata) if !metadata.file_type().is_file() => {
            Err(FilePersistenceError::NotARegularFile(path))
        }
        Ok(_) => {
            let mut file = fs::File::open(path)?;
            let mut bytes = Vec::new();
            std::io::Read::by_ref(&mut file)
                .take(MAX_NATIVE_DOCUMENT_BYTES as u64 + 61)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_NATIVE_DOCUMENT_BYTES + 60 {
                return Err(FilePersistenceError::Format(
                    PersistenceError::ResourceLimit,
                ));
            }
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let identity = FileIdentity {
                byte_len: u64_len(bytes.len()).map_err(FilePersistenceError::Format)?,
                sha256: hasher.clone().finalize().into(),
            };
            if expected == Some(identity) {
                Ok(Some(ObservedWorkRecovery { bytes, hasher }))
            } else {
                Err(FilePersistenceError::ExternalConflict)
            }
        }
    }
}

/// Appends the step that brings the checkpoint of `path`, observed as
/// `expected`, to `document` (idea #49). `None` when the checkpoint must be
/// written whole. The same four identity checks guard the append as a whole write.
pub(super) fn append_work_recovery_step(
    path: &Path,
    document: &DocumentStore,
    container: &ContainerData,
    base_identity: FileIdentity,
    expected: FileIdentity,
) -> Result<Option<FileIdentity>, FilePersistenceError> {
    let _lock = lock_saves(path)?;
    if read_native_document_identity(path)? != base_identity {
        return Err(FilePersistenceError::ExternalConflict);
    }
    let Some(observed) = read_observed_work_recovery(path, Some(expected))? else {
        return Err(FilePersistenceError::ExternalConflict);
    };
    let Some(step) =
        work_recovery_steps::plan_step(&observed.bytes, document, container, base_identity)
    else {
        return Ok(None);
    };
    if read_native_document_identity(path)? != base_identity {
        return Err(FilePersistenceError::ExternalConflict);
    }
    check_work_recovery_identity(path, Some(expected))?;
    let format = FilePersistenceError::Format;
    let mut framed = Vec::with_capacity(8 + step.len());
    push_u64(&mut framed, u64_len(step.len()).map_err(format)?);
    framed.extend_from_slice(&step);
    let recovery = work_recovery_path(path);
    let observed_len = u64_len(observed.bytes.len()).map_err(format)?;
    let mut file = fs::OpenOptions::new().write(true).open(&recovery)?;
    let appended = std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(observed_len))
        .and_then(|_| file.write_all(&framed))
        .and_then(|()| file.sync_all());
    if let Err(error) = appended {
        // A step cut short would hide every later one: the checkpoint stays as
        // this writer observed it, so the edit can be retried.
        let _ = file.set_len(observed_len).and_then(|()| file.sync_all());
        return Err(FilePersistenceError::Io(error));
    }
    let mut hasher = observed.hasher;
    hasher.update(&framed);
    let identity = FileIdentity {
        byte_len: u64_len(observed.bytes.len() + framed.len()).map_err(format)?,
        sha256: hasher.finalize().into(),
    };
    let parent = recovery
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    sync_parent_directory(parent).map_err(|error| FilePersistenceError::Published {
        path: recovery.clone(),
        identity,
        error,
    })?;
    Ok(Some(identity))
}

/// Moves a work-recovery checkpoint the open could not use (see
/// [`LoadNotice::WorkRecoveryUnusable`](super::LoadNotice)) to the first free
/// `<file>.work-recovery.stale`, `.stale-2`, ... and returns that path. The work
/// in it stays on disk, and the session that holds `_owner` can write its own
/// checkpoint again.
pub fn set_aside_work_recovery(
    path: &Path,
    _owner: &WorkRecoveryLock,
) -> Result<PathBuf, FilePersistenceError> {
    let _lock = lock_saves(path)?;
    let source = work_recovery_path(path);
    for index in 1..=1000_u32 {
        let mut target = source.as_os_str().to_os_string();
        target.push(if index == 1 {
            ".stale".to_owned()
        } else {
            format!(".stale-{index}")
        });
        let target = PathBuf::from(target);
        match fs::symlink_metadata(&target) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(FilePersistenceError::Io(error)),
        }
        fs::rename(&source, &target)?;
        sync_parent_directory(
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        return Ok(target);
    }
    Err(FilePersistenceError::SetAsideNamesTaken(source))
}

#[cfg(feature = "testing")]
thread_local! {
    pub(super) static FAIL_PARENT_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Inject a directory-sync error on this thread in the real publication path.
/// This tests publication semantics, not the platform's fsync implementation.
#[cfg(feature = "testing")]
pub fn headless_with_parent_sync_failure<T>(operation: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FAIL_PARENT_SYNC.set(self.0);
        }
    }
    let _reset = Reset(FAIL_PARENT_SYNC.replace(true));
    operation()
}
