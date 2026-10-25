use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use sha2::{Digest, Sha256};

use super::work_recovery_steps::CheckpointPlan;
use super::*;

/// A file as this process last read or wrote it: its identity, the SHA-256
/// state of its bytes (an appended step is hashed alone) and its step plan.
#[derive(Clone)]
struct Observed {
    identity: FileIdentity,
    hasher: Sha256,
    plan: Option<CheckpointPlan>,
}

impl Observed {
    fn new(byte_len: u64, hasher: Sha256, plan: Option<CheckpointPlan>) -> Self {
        let sha256 = hasher.clone().finalize().into();
        let identity = FileIdentity { byte_len, sha256 };
        Self {
            identity,
            hasher,
            plan,
        }
    }
}

fn observed_of(bytes: &[u8]) -> Observed {
    crate::graph::note_hashed(bytes.len());
    let byte_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let plan = work_recovery_steps::read_plan(bytes);
    Observed::new(byte_len, Sha256::new_with_prefix(bytes), plan)
}

/// Length, modified time and, on Unix, inode and change time: while they stay,
/// an edit does not read and hash the file again (review 2026-10-21 LIM-1). An
/// in-place rewrite keeping length and times is missed; save locks bar sessions.
type Fingerprint = (u64, Option<std::time::SystemTime>, (u64, i64, i64));

fn fingerprint(metadata: &fs::Metadata) -> Fingerprint {
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        (metadata.ino(), metadata.ctime(), metadata.ctime_nsec())
    };
    #[cfg(not(unix))]
    let inode = (0, 0, 0);
    (metadata.len(), metadata.modified().ok(), inode)
}

fn observed_files() -> MutexGuard<'static, HashMap<PathBuf, (Fingerprint, Observed)>> {
    static FILES: LazyLock<Mutex<HashMap<PathBuf, (Fingerprint, Observed)>>> =
        LazyLock::new(Mutex::default);
    FILES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The file at `path`, read only when its fingerprint changed since this
/// process last read or wrote it.
fn observe(path: &Path) -> Result<Observed, FilePersistenceError> {
    let mut file = fs::File::open(path)?;
    let before = fingerprint(&file.metadata()?);
    if let Some((seen, observed)) = observed_files().get(path)
        && *seen == before
    {
        return Ok(observed.clone());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let observed = observed_of(&bytes);
    // A file written while it was read is read again next time.
    if fingerprint(&file.metadata()?) == before {
        observed_files().insert(path.to_path_buf(), (before, observed.clone()));
    }
    Ok(observed)
}

/// Records `observed` as what this process just wrote at `path`; its identity.
fn record_written(path: &Path, observed: Observed) -> FileIdentity {
    let identity = observed.identity;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        observed_files().insert(path.to_path_buf(), (fingerprint(&metadata), observed));
    }
    identity
}

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

    /// [`Self::acquire`], or `None` where the file's folder cannot take the lock
    /// file at all (a read-only share, a protected USB key, a mounted DMG). No
    /// checkpoint can be written there either, so the file opens without one.
    ///
    /// A lock file that cannot be written in a folder that takes files (another user's
    /// on a shared drive, one marked read-only by an unpacked archive) is locked through
    /// a read-only handle instead, so a session holding it still keeps this one out
    /// (review 2026-10-24 UI-21). Where no lock can be taken through such a handle,
    /// the file opens without a checkpoint as well.
    pub fn acquire_unless_read_only(path: &Path) -> Result<Option<Self>, FilePersistenceError> {
        let denied = match Self::acquire(path) {
            Err(FilePersistenceError::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
                ) =>
            {
                error
            }
            acquired => return acquired.map(Some),
        };
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".work-recovery-lock");
        let held = match fs::File::open(lock_path) {
            Ok(file) => match read_handle_lock(&file) {
                Ok(()) => Some(Some(Self { _file: file })),
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(FilePersistenceError::ExternalConflict);
                }
                // A Linux NFS client locks only a file opened for writing (EBADF).
                Err(std::fs::TryLockError::Error(_)) => Some(None),
            },
            Err(_) => None,
        };
        if !folder_takes_files(path) {
            return Ok(None);
        }
        held.ok_or(FilePersistenceError::Io(denied))
    }
}

fn read_handle_lock(file: &fs::File) -> Result<(), std::fs::TryLockError> {
    #[cfg(feature = "testing")]
    if READ_HANDLE_LOCK_FAILS.get() {
        return Err(std::fs::TryLockError::Error(io::Error::from_raw_os_error(
            9,
        )));
    }
    file.try_lock()
}

/// Checks the observed checkpoint while the caller holds its WorkRecoveryLock.
pub fn check_work_recovery_identity(
    path: &Path,
    expected: Option<FileIdentity>,
) -> Result<(), FilePersistenceError> {
    read_observed_work_recovery(path, expected).map(drop)
}

/// The checkpoint [`check_work_recovery_identity`] found as `expected`, read
/// only when its metadata changed since this process last read or wrote it.
fn read_observed_work_recovery(
    path: &Path,
    expected: Option<FileIdentity>,
) -> Result<Option<Observed>, FilePersistenceError> {
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
            let observed = observe(&path)?;
            if expected == Some(observed.identity) {
                Ok(Some(observed))
            } else {
                Err(FilePersistenceError::ExternalConflict)
            }
        }
    }
}

/// Writes `document` as the work-recovery checkpoint of the file at `path` (see
/// [`publish_work_recovery`]). Over the checkpoint it observed as `expected` it
/// appends only the step to `document` where a step can name it (idea #49), and
/// otherwise writes the checkpoint whole; the same identity checks guard both.
pub(super) fn write_work_recovery(
    path: &Path,
    document: &DocumentStore,
    container: &ContainerData,
    base_identity: FileIdentity,
    expected: Option<FileIdentity>,
    after_compare: impl FnOnce(),
) -> Result<FileIdentity, FilePersistenceError> {
    let _lock = lock_saves(path)?;
    if observe(path)?.identity != base_identity {
        return Err(FilePersistenceError::ExternalConflict);
    }
    let observed = read_observed_work_recovery(path, expected)?;
    after_compare();
    let unchanged = || {
        if observe(path)?.identity != base_identity {
            return Err(FilePersistenceError::ExternalConflict);
        }
        check_work_recovery_identity(path, expected)
    };
    if let Some(observed) = observed
        && let Some(plan) = &observed.plan
        && let Some((step, next)) =
            work_recovery_steps::plan_step(plan, document, container, base_identity)
    {
        unchanged()?;
        return append_step(path, observed, &step, next);
    }
    // The checkpoint keeps the Undo history the file does not, unless it does not fit.
    let payload = &match save_document_store(document, container) {
        Err(PersistenceError::ResourceLimit | PersistenceError::TooLarge { .. }) => {
            save_document_store_current_snapshot(document, container)
        }
        saved => saved,
    }
    .map_err(FilePersistenceError::Format)?;
    let mut bytes = Vec::with_capacity(WORK_RECOVERY_MAGIC.len() + 2 + 8 + 32 + 8 + payload.len());
    bytes.extend_from_slice(WORK_RECOVERY_MAGIC);
    push_u16(&mut bytes, WORK_RECOVERY_SCHEMA);
    push_u64(&mut bytes, base_identity.byte_len());
    bytes.extend_from_slice(&base_identity.sha256());
    push_u64(
        &mut bytes,
        u64_len(payload.len()).map_err(FilePersistenceError::Format)?,
    );
    bytes.extend_from_slice(payload);
    let recovery = work_recovery_path(path);
    write_atomic_after_prepare(&recovery, &bytes, true, unchanged, sync_parent_directory)?;
    Ok(record_written(&recovery, observed_of(&bytes)))
}

/// Appends `step` to the checkpoint of `path`, which the caller observed and
/// rechecked under the save lock.
fn append_step(
    path: &Path,
    observed: Observed,
    step: &[u8],
    next: CheckpointPlan,
) -> Result<FileIdentity, FilePersistenceError> {
    let format = FilePersistenceError::Format;
    let mut framed = Vec::with_capacity(8 + step.len());
    push_u64(&mut framed, u64_len(step.len()).map_err(format)?);
    framed.extend_from_slice(step);
    let recovery = work_recovery_path(path);
    let observed_len = observed.identity.byte_len;
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
    crate::graph::note_hashed(framed.len());
    hasher.update(&framed);
    let byte_len = observed_len + u64_len(framed.len()).map_err(format)?;
    let identity = record_written(&recovery, Observed::new(byte_len, hasher, Some(next)));
    let parent = recovery
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    sync_parent_directory(parent).map_err(|error| FilePersistenceError::Published {
        path: recovery.clone(),
        identity,
        error,
    })?;
    Ok(identity)
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

/// Whether the folder of `path` takes a new file: a probe file is made there and removed.
fn folder_takes_files(path: &Path) -> bool {
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    #[cfg(feature = "testing")]
    if READ_ONLY_FOLDER.with_borrow(|read_only| read_only.as_deref() == Some(folder)) {
        return false;
    }
    tempfile::Builder::new()
        .prefix(".ketchup-write-probe")
        .tempfile_in(folder)
        .is_ok()
}

#[cfg(feature = "testing")]
thread_local! {
    pub(super) static FAIL_PARENT_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static READ_ONLY_FOLDER: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static READ_HANDLE_LOCK_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `operation` with a lock through a read-only handle failing on this thread, as
/// on a Linux NFS client, which locks only a file opened for writing (EBADF).
#[cfg(feature = "testing")]
pub fn headless_with_read_handle_lock_failure<T>(operation: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            READ_HANDLE_LOCK_FAILS.set(self.0);
        }
    }
    let _reset = Reset(READ_HANDLE_LOCK_FAILS.replace(true));
    operation()
}

/// Runs `operation` with `folder` taking no new file on this thread, as a read-only share
/// does; a real one cannot be made in a test on every platform (Windows ignores a
/// folder's read-only flag, root ignores its mode).
#[cfg(feature = "testing")]
pub fn headless_with_read_only_folder<T>(folder: &Path, operation: impl FnOnce() -> T) -> T {
    struct Reset(Option<PathBuf>);
    impl Drop for Reset {
        fn drop(&mut self) {
            READ_ONLY_FOLDER.set(self.0.take());
        }
    }
    let _reset = Reset(READ_ONLY_FOLDER.replace(Some(folder.to_path_buf())));
    operation()
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
