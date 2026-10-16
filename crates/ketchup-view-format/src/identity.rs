//! The export identity of a package, which binds comments to it: the SHA-256
//! of its manifest and geometry. Hashing a large package takes long, so an
//! opened package keeps the identity it was read with.

use crate::{Comments, Package};
use ketchup_rejection::Rejection;
use std::io::{Read, Write};

/// A package and its export identity, as [`Comments`] are bound to it.
pub trait PackageExport {
    fn package(&self) -> &Package;
    /// SHA-256 of the manifest and geometry; see [`Package::payload_sha256`].
    fn payload_digest(&self) -> Result<[u8; 32], Rejection>;
}

impl PackageExport for Package {
    fn package(&self) -> &Package {
        self
    }

    fn payload_digest(&self) -> Result<[u8; 32], Rejection> {
        self.digest()
    }
}

/// A package with its export identity computed once. Its contents cannot be
/// changed, so the identity stays theirs.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentifiedPackage {
    package: Package,
    digest: [u8; 32],
}

impl IdentifiedPackage {
    /// Hash `package` once.
    pub fn new(package: Package) -> Result<Self, Rejection> {
        let digest = package.digest()?;
        Ok(Self { package, digest })
    }

    pub(crate) const fn known(package: Package, digest: [u8; 32]) -> Self {
        Self { package, digest }
    }

    /// Read a package and the comments saved into this copy, if any; the
    /// identity is the checksum already verified when it was written by
    /// [`Package::write`], so the geometry is hashed once.
    pub fn read_with_comments(reader: impl Read) -> Result<(Self, Option<Comments>), Rejection> {
        Package::read_identified(reader)
    }

    pub const fn package(&self) -> &Package {
        &self.package
    }

    pub fn into_package(self) -> Package {
        self.package
    }

    /// The identity as [`Package::payload_sha256`] gives it, without hashing.
    pub fn payload_sha256(&self) -> String {
        hex(&self.digest)
    }

    /// [`Package::write`] with the identity as its checksum, without hashing.
    pub fn write(&self, writer: impl Write) -> Result<(), Rejection> {
        self.package.write_with_digest(writer, Some(self.digest))
    }

    /// [`Package::write_with_comments`] without hashing the geometry again.
    pub fn write_with_comments(
        &self,
        writer: impl Write,
        comments: &Comments,
    ) -> Result<(), Rejection> {
        self.package.write_commented(writer, comments, self)
    }
}

impl PackageExport for IdentifiedPackage {
    fn package(&self) -> &Package {
        &self.package
    }

    fn payload_digest(&self) -> Result<[u8; 32], Rejection> {
        Ok(self.digest)
    }
}

pub(crate) fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
