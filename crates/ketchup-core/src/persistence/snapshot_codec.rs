//! Native snapshot encoding.
//!
//! The product model has one serde definition; this module stores it as CBOR behind the
//! `KETCHUPDOC` magic, a format number and a SHA-256 checksum of the payload. New fields
//! declare `#[serde(default)]` and need no new format number. A new number is needed only
//! when an existing field changes meaning; its conversion then goes into [`decode`].

use super::{MAGIC, PersistenceError};
use crate::document::{ProductModel, Snapshot};

/// Format number written after the magic. Numbers below it belong to the retired
/// hand-written codec.
pub(super) const SNAPSHOT_FORMAT: u16 = 98;

const CHECKSUM_BYTES: usize = 32;

#[derive(serde::Serialize)]
struct EncodedSnapshot<'a> {
    revision_id: u64,
    product: &'a ProductModel,
}

#[derive(serde::Deserialize)]
struct DecodedSnapshot {
    revision_id: u64,
    product: ProductModel,
}

pub(super) fn encode(snapshot: &Snapshot) -> Vec<u8> {
    let mut payload = Vec::new();
    ciborium::into_writer(
        &EncodedSnapshot {
            revision_id: snapshot.revision_id(),
            product: snapshot.product(),
        },
        &mut payload,
    )
    .expect("the product model serializes into memory");
    let mut bytes = Vec::with_capacity(MAGIC.len() + 2 + CHECKSUM_BYTES + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&SNAPSHOT_FORMAT.to_le_bytes());
    bytes.extend_from_slice(&crate::graph::sha256_bytes(&payload));
    bytes.extend_from_slice(&payload);
    bytes
}

/// Rewrites the stored snapshot as a CBOR value tree and reseals its checksum.
#[cfg(feature = "testing")]
pub(crate) fn rewrite(bytes: &[u8], edit: impl FnOnce(&mut ciborium::Value)) -> Vec<u8> {
    let header = MAGIC.len() + 2;
    let mut value: ciborium::Value =
        ciborium::from_reader(&bytes[header + CHECKSUM_BYTES..]).expect("a saved snapshot");
    edit(&mut value);
    let mut payload = Vec::new();
    ciborium::into_writer(&value, &mut payload).expect("a CBOR value serializes into memory");
    let mut rewritten = bytes[..header].to_vec();
    rewritten.extend_from_slice(&crate::graph::sha256_bytes(&payload));
    rewritten.extend_from_slice(&payload);
    rewritten
}

/// Decodes the bytes that follow the magic and the format number.
pub(super) fn decode(body: &[u8]) -> Result<(u64, ProductModel), PersistenceError> {
    let (checksum, payload) = body
        .split_at_checked(CHECKSUM_BYTES)
        .ok_or(PersistenceError::Truncated)?;
    if crate::graph::sha256_bytes(payload) != checksum {
        return Err(PersistenceError::ChecksumMismatch);
    }
    let decoded: DecodedSnapshot = ciborium::from_reader(payload)
        .map_err(|error| PersistenceError::InvalidPayload(error.to_string()))?;
    Ok((decoded.revision_id, decoded.product))
}
