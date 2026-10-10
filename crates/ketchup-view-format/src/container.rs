//! The binary `.ketchup-view` envelope: header, size limits and checksum.

use crate::{Manifest, glb, reject};
use ketchup_rejection::Rejection;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

/// File signature; the first 11 bytes of every package.
const MAGIC: &[u8; 11] = b"KETCHUPVIEW";
/// The only package version this build reads and writes.
const VERSION: u16 = 1;
/// Magic, version, two payload lengths and the SHA-256 of both payloads.
const HEADER_BYTES: usize = 11 + 2 + 4 + 4 + 32;
/// Largest accepted manifest JSON.
pub const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
/// Largest accepted embedded GLB.
pub const MAX_GLB_BYTES: usize = 256 * 1024 * 1024;

/// One `.ketchup-view` package: the manifest and its embedded display GLB.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub manifest: Manifest,
    pub glb: Vec<u8>,
}

impl Package {
    /// Revalidates public package fields before exposing borrowed renderer data.
    pub fn geometry(&self) -> Result<crate::Geometry<'_>, Rejection> {
        let (geometry, info) = glb::geometry::decode(&self.glb)?;
        self.manifest.validate(&info)?;
        Ok(geometry)
    }

    /// Read and fully validate a package; size limits are checked before allocating.
    pub fn read(mut reader: impl Read) -> Result<Self, Rejection> {
        let mut header = [0; HEADER_BYTES];
        reader.read_exact(&mut header).map_err(|error| {
            reject("header", "The package header is incomplete.").caused_by(error)
        })?;
        if &header[..11] != MAGIC || u16::from_le_bytes([header[11], header[12]]) != VERSION {
            return Err(reject(
                "version",
                "Unknown Viewer package magic or version.",
            ));
        }
        let manifest_len = length(&header[13..17], MAX_MANIFEST_BYTES, "manifest")?;
        let glb_len = length(&header[17..21], MAX_GLB_BYTES, "glb")?;
        // Read incrementally; a forged size does not eagerly allocate hundreds of MiB.
        let mut payload = Vec::new();
        let total = manifest_len + glb_len;
        reader
            .by_ref()
            .take(u64::try_from(total).expect("bounded size fits u64"))
            .read_to_end(&mut payload)
            .map_err(|error| {
                reject("payload", "The package payload could not be read.").caused_by(error)
            })?;
        if payload.len() != total {
            return Err(reject("payload", "The package payload is truncated."));
        }
        let mut trailing = [0];
        if reader.read(&mut trailing).map_err(|error| {
            reject("payload", "The package end could not be read.").caused_by(error)
        })? != 0
        {
            return Err(reject(
                "payload",
                "Trailing data is not part of a Viewer package.",
            ));
        }
        if Sha256::digest(&payload)[..] != header[21..] {
            return Err(reject("checksum", "The package contents are damaged."));
        }
        let manifest: Manifest =
            serde_json::from_slice(&payload[..manifest_len]).map_err(|error| {
                reject("manifest", "The package manifest is malformed.").caused_by(error)
            })?;
        let info = glb::validate(&payload[manifest_len..])?;
        manifest.validate(&info)?;
        Ok(Self {
            manifest,
            glb: payload.split_off(manifest_len),
        })
    }

    /// Validate, then write the package; nothing is written when validation fails.
    pub fn write(&self, mut writer: impl Write) -> Result<(), Rejection> {
        if self.glb.len() > MAX_GLB_BYTES {
            return Err(reject("glb", "Geometry exceeds the package size limit."));
        }
        let info = glb::validate(&self.glb)?;
        self.manifest.validate(&info)?;
        // A bounded writer also stops oversized author-side metadata serialization.
        let mut json = LimitedJson(Vec::new());
        serde_json::to_writer(&mut json, &self.manifest).map_err(|error| {
            reject(
                "manifest",
                "The manifest could not be encoded within its size limit.",
            )
            .caused_by(error)
        })?;
        let json = json.0;
        let mut checksum = Sha256::new();
        checksum.update(&json);
        checksum.update(&self.glb);
        let mut header = Vec::with_capacity(HEADER_BYTES);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&VERSION.to_le_bytes());
        header.extend_from_slice(
            &u32::try_from(json.len())
                .expect("8 MiB fits u32")
                .to_le_bytes(),
        );
        header.extend_from_slice(
            &u32::try_from(self.glb.len())
                .expect("256 MiB fits u32")
                .to_le_bytes(),
        );
        header.extend_from_slice(&checksum.finalize());
        writer
            .write_all(&header)
            .and_then(|()| writer.write_all(&json))
            .and_then(|()| writer.write_all(&self.glb))
            .map_err(|error| {
                reject("output", "The Viewer package could not be written.").caused_by(error)
            })
    }
}

/// A little-endian u32 payload length, refused above `limit`.
fn length(bytes: &[u8], limit: usize, target: &str) -> Result<usize, Rejection> {
    let value = u32::from_le_bytes(bytes.try_into().expect("header length field"));
    usize::try_from(value)
        .ok()
        .filter(|&len| len > 0 && len <= limit)
        .ok_or_else(|| {
            reject(
                target,
                "The declared length is empty or exceeds the package limit.",
            )
        })
}

/// A JSON sink that refuses output beyond the manifest limit.
struct LimitedJson(Vec<u8>);

impl Write for LimitedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_MANIFEST_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("manifest exceeds 8 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
