//! Native snapshot encoding.
//!
//! The product model has one serde definition; this module stores it as CBOR behind the
//! `KETCHUPDOC` magic, a format number and a SHA-256 checksum of the payload. New fields
//! declare `#[serde(default)]` and need no new format number. A new number is needed only
//! when an existing field changes name or meaning; its conversion then goes into [`decode`].

use super::{MAGIC, PersistenceError};
use crate::document::{ProductModel, Snapshot};

/// Format number written after the magic.
pub(super) const SNAPSHOT_FORMAT: u16 = 99;

/// First format this codec wrote. Numbers below it belong to the retired hand-written
/// codec.
pub(super) const FIRST_SNAPSHOT_FORMAT: u16 = 98;

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

/// A decoded snapshot, and for an older format the digest its writer computed.
pub(super) struct Decoded {
    pub(super) revision_id: u64,
    pub(super) product: ProductModel,
    pub(super) writer_digest: Option<String>,
}

/// Decodes the bytes that follow the magic and a format number of this codec.
pub(super) fn decode(format: u16, body: &[u8]) -> Result<Decoded, PersistenceError> {
    let (checksum, payload) = body
        .split_at_checked(CHECKSUM_BYTES)
        .ok_or(PersistenceError::Truncated)?;
    if crate::graph::sha256_bytes(payload) != checksum {
        return Err(PersistenceError::ChecksumMismatch);
    }
    let invalid =
        |error: &dyn std::fmt::Display| PersistenceError::InvalidPayload(error.to_string());
    if format == SNAPSHOT_FORMAT {
        let decoded: DecodedSnapshot =
            ciborium::from_reader(payload).map_err(|error| invalid(&error))?;
        return Ok(Decoded {
            revision_id: decoded.revision_id,
            product: decoded.product,
            writer_digest: None,
        });
    }
    let mut value: ciborium::Value =
        ciborium::from_reader(payload).map_err(|error| invalid(&error))?;
    if let Some(product) = field(&mut value, "product") {
        rename_format_98_fields(product, |(old, new)| (old, new));
    }
    let decoded: DecodedSnapshot = value.deserialized().map_err(|error| invalid(&error))?;
    // The writer hashed the identity form under its own field names.
    let mut writer_form =
        crate::document::identity_form(|| ciborium::Value::serialized(&decoded.product))
            .map_err(|error| invalid(&error))?;
    rename_format_98_fields(&mut writer_form, |(old, new)| (new, old));
    Ok(Decoded {
        revision_id: decoded.revision_id,
        writer_digest: Some(crate::document::digest_product(&writer_form)),
        product: decoded.product,
    })
}

/// Format 99 names the generic pin joint: the product's `dowel_joints` became
/// `pin_joints`, a joint's `dowel` its `pin`, and a recipe joinery item's
/// `dowel_joint_id` its `pin_joint_id`. Values are unchanged. `direction` maps an
/// (old, new) pair to the (from, to) names of this pass.
fn rename_format_98_fields(
    product: &mut ciborium::Value,
    direction: impl Fn((&'static str, &'static str)) -> (&'static str, &'static str),
) {
    let (joints, joint_spec, recipe_joint) = (
        direction(("dowel_joints", "pin_joints")),
        direction(("dowel", "pin")),
        direction(("dowel_joint_id", "pin_joint_id")),
    );
    if let Some(ciborium::Value::Map(entries)) = rename_field(product, joints.0, joints.1) {
        for (_, joint) in entries {
            rename_field(joint, joint_spec.0, joint_spec.1);
        }
    }
    if let Some(ciborium::Value::Map(joinery)) =
        field(product, "assembly_recipe").and_then(|recipe| field(recipe, "joinery"))
    {
        for (_, item) in joinery {
            rename_field(item, recipe_joint.0, recipe_joint.1);
        }
    }
}

fn field<'a>(value: &'a mut ciborium::Value, name: &str) -> Option<&'a mut ciborium::Value> {
    let ciborium::Value::Map(entries) = value else {
        return None;
    };
    entries
        .iter_mut()
        .find(|(key, _)| key.as_text() == Some(name))
        .map(|(_, value)| value)
}

fn rename_field<'a>(
    value: &'a mut ciborium::Value,
    from: &str,
    to: &str,
) -> Option<&'a mut ciborium::Value> {
    let ciborium::Value::Map(entries) = value else {
        return None;
    };
    let (key, value) = entries
        .iter_mut()
        .find(|(key, _)| key.as_text() == Some(from))?;
    *key = ciborium::Value::Text(to.to_owned());
    Some(value)
}
