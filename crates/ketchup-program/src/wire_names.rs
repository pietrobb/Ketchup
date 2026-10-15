//! Names a report carries as `&'static str` (issue kinds, statuses, check
//! names), read back when the report comes from the program worker. Each
//! distinct name is kept once for the life of the process.
use serde::{Deserialize, Deserializer, de::Error};
use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

/// Far more distinct names than the checks use; a reader past it refuses
/// instead of keeping more.
const MAX_NAMES: usize = 1024;
const MAX_NAME_BYTES: usize = 64;

/// `&'static str` under another name: serde's derive borrows a field written
/// `&'static str` from its input, which ties the reader to `'static` input.
pub(crate) type Name = &'static str;

static NAMES: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());

fn intern<E: Error>(name: String) -> Result<&'static str, E> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(E::custom(format!("{name:?} is not a report name")));
    }
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(known) = names.get(name.as_str()) {
        return Ok(known);
    }
    if names.len() >= MAX_NAMES {
        return Err(E::custom(format!(
            "a report may use at most {MAX_NAMES} distinct names"
        )));
    }
    let kept: &'static str = Box::leak(name.into_boxed_str());
    names.insert(kept);
    Ok(kept)
}

pub(crate) fn name<'de, D: Deserializer<'de>>(reader: D) -> Result<&'static str, D::Error> {
    intern(String::deserialize(reader)?)
}

/// Reads a name the way [`name`] does, for callers outside a derived reader.
pub(crate) fn known<E: Error>(name: String) -> Result<&'static str, E> {
    intern(name)
}
