//! Saved fields that do not identify the document.
//!
//! A canonical digest hashes the serde form of a value, the same definition the native file
//! stores. A saved field that is evidence of an evaluation or is derived from other fields
//! declares `#[serde(serialize_with = "ketchup_geometry::derived::derived")]`; inside
//! [`identity_form`] it serializes as a unit, so it never changes a digest.

use serde::Serialize;
use std::cell::Cell;

thread_local! {
    static HASHING_IDENTITY: Cell<bool> = const { Cell::new(false) };
}

/// Serializes a saved field that does not identify the document.
pub fn derived<T: Serialize, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if HASHING_IDENTITY.get() {
        serializer.serialize_unit()
    } else {
        value.serialize(serializer)
    }
}

/// Runs `serialize` so that every derived field serializes as a unit.
pub fn identity_form<R>(serialize: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            HASHING_IDENTITY.set(self.0);
        }
    }
    let _restore = Restore(HASHING_IDENTITY.replace(true));
    serialize()
}
