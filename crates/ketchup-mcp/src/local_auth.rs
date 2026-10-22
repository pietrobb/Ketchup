//! OS-protected bootstrap files. No secret is written before permissions are sealed.
//!
//! The bootstrap itself never crosses the attach socket: the client and the
//! window each prove they know it (`attach_proof`), so a program that took the
//! address of a closed window learns nothing and cannot pass for the window.
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

/// The attach request's proof: bound to the window it is for and the request's nonce.
#[must_use]
pub fn client_attach_proof(bootstrap: &str, instance_id: &str, nonce: &str) -> String {
    attach_proof(bootstrap, "client", &[instance_id, nonce])
}

/// The list request's proof. Its own role keeps it from passing as an attach.
#[must_use]
pub fn client_list_proof(bootstrap: &str, instance_id: &str, nonce: &str) -> String {
    attach_proof(bootstrap, "client-list", &[instance_id, nonce])
}

/// The window's proof with its attach answer, over the instance id, nonce,
/// status, bridge address and token it answers (empty when absent).
#[must_use]
pub fn window_attach_proof(bootstrap: &str, answer: &[&str; 5]) -> String {
    attach_proof(bootstrap, "window", answer)
}

/// The window's proof with its list answer, over the instance id, nonce,
/// status and document name it answers, so a program that took the address of
/// a closed window cannot name a document for it.
#[must_use]
pub fn window_list_proof(bootstrap: &str, answer: &[&str; 4]) -> String {
    attach_proof(bootstrap, "window-list", answer)
}

/// HMAC-SHA256 (RFC 2104) keyed with `bootstrap` over `role` and the
/// newline-separated `fields`, in lower hex.
fn attach_proof(bootstrap: &str, role: &str, fields: &[&str]) -> String {
    const BLOCK: usize = 64;
    let mut key = [0_u8; BLOCK];
    if bootstrap.len() > BLOCK {
        key[..32].copy_from_slice(&Sha256::digest(bootstrap.as_bytes()));
    } else {
        key[..bootstrap.len()].copy_from_slice(bootstrap.as_bytes());
    }
    let mut inner = Sha256::new();
    inner.update(key.map(|byte| byte ^ 0x36));
    inner.update(role.as_bytes());
    for field in fields {
        inner.update(b"\n");
        inner.update(field.as_bytes());
    }
    let mut outer = Sha256::new();
    outer.update(key.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    outer
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `supplied` is `expected`, compared in time independent of where they differ.
#[must_use]
pub fn proofs_match(supplied: &str, expected: &str) -> bool {
    supplied.len() == expected.len()
        && supplied
            .bytes()
            .zip(expected.bytes())
            .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

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

    #[test]
    fn attach_proof_is_hmac_sha256_of_role_and_fields() {
        // RFC 4231, test case 2.
        assert_eq!(
            attach_proof("Jefe", "what do ya want for nothing?", &[]),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        let proof = attach_proof(&"b".repeat(64), "client", &["id", "nonce"]);
        assert_eq!(
            proof,
            attach_proof(&"b".repeat(64), "client\nid\nnonce", &[])
        );
        assert_ne!(
            proof,
            attach_proof(&"b".repeat(64), "window", &["id", "nonce"])
        );
        assert!(proofs_match(&proof, &proof.clone()));
        assert!(!proofs_match(&proof[..63], &proof));
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
