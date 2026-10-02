//! OS-protected bootstrap files. No secret is written before permissions are sealed.
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "local_auth_windows.rs"]
mod platform;

#[cfg(unix)]
#[allow(unsafe_code)]
mod platform {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    pub fn create(path: &Path) -> io::Result<File> {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    pub fn validate(file: &File) -> io::Result<()> {
        let m = file.metadata()?;
        // SAFETY: geteuid takes no pointers and has no preconditions.
        let uid = unsafe { libc::geteuid() };
        if !m.is_file() || m.uid() != uid || m.mode() & 0o077 != 0 || m.nlink() != 1 {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(())
    }
}

/// Atomically create a private file: even an early opener cannot retain read access.
pub fn create(path: &Path) -> io::Result<File> {
    platform::create(path)
}

/// Read only a regular, current-user-owned private file, validating the opened
/// handle rather than trusting path metadata. Bounds apply before allocation.
pub fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    platform::validate(&file)?;
    let mut bytes = Vec::new();
    const MAX_BOOTSTRAP_BYTES: usize = 512;
    file.take((MAX_BOOTSTRAP_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BOOTSTRAP_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

#[cfg(not(any(windows, unix)))]
mod platform {
    use super::*;
    pub fn create(_: &Path) -> io::Result<File> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub fn validate(_: &File) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn private_bootstrap_roundtrip_and_bounds() {
        let mut file = tempfile::Builder::new().make(create).unwrap();
        file.write_all(b"private bootstrap").unwrap();
        assert_eq!(read_private(file.path()).unwrap(), b"private bootstrap");
        file.write_all(&[0; 513]).unwrap();
        assert!(read_private(file.path()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn public_permissions_and_symlinks_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        let file = create(&path).unwrap();
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(read_private(&link).is_err());
        file.set_permissions(std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(read_private(&path).is_err());
    }
}
