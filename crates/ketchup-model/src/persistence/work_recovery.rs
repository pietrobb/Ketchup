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
    let path = work_recovery_path(path);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound && expected.is_none() => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(FilePersistenceError::ExternalConflict)
        }
        Err(error) => Err(FilePersistenceError::Io(error)),
        Ok(metadata) if !metadata.file_type().is_file() => {
            Err(FilePersistenceError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "work-recovery path is not a regular file",
            )))
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
            if expected == Some(FileIdentity::from_bytes(&bytes)) {
                Ok(())
            } else {
                Err(FilePersistenceError::ExternalConflict)
            }
        }
    }
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
