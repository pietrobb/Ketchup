//! Each digest format has one writer and one checker; the checker accepts exactly what
//! the writer produces.

use ketchup_geometry::reference::{fnv1a64_fingerprint, is_fnv1a64_fingerprint};
use ketchup_model::document::{DocumentStore, Snapshot};
use ketchup_model::graph::{is_sha256_hex, sha256_hex};

#[test]
fn sha256_checker_accepts_exactly_the_written_form() {
    let digest = sha256_hex(b"ketchup");
    assert!(is_sha256_hex(&digest));
    assert!(!is_sha256_hex(&format!("A{}", &digest[1..])));
    assert!(!is_sha256_hex(&digest[1..]));
    assert!(!is_sha256_hex(&format!("{digest}0")));
    assert!(!is_sha256_hex(&digest.replacen(&digest[..1], "g", 1)));
}

#[test]
fn snapshot_digest_checker_accepts_exactly_the_written_form() {
    let digest = DocumentStore::new().current().canonical_digest();
    assert!(Snapshot::is_canonical_digest(&digest));
    assert!(!Snapshot::is_canonical_digest(&format!(
        "A{}",
        &digest[1..]
    )));
    assert!(!Snapshot::is_canonical_digest(&digest[1..]));
    assert!(!Snapshot::is_canonical_digest(&sha256_hex(b"ketchup")));
}

#[test]
fn fnv1a64_checker_accepts_exactly_the_written_form() {
    let fingerprint = fnv1a64_fingerprint("ketchup");
    assert!(is_fnv1a64_fingerprint(&fingerprint));
    assert_ne!(fingerprint, fnv1a64_fingerprint("ketchup "));
    assert!(!is_fnv1a64_fingerprint(&format!(
        "{}A",
        &fingerprint[..fingerprint.len() - 1]
    )));
    assert!(!is_fnv1a64_fingerprint(
        &fingerprint[..fingerprint.len() - 1]
    ));
    assert!(!is_fnv1a64_fingerprint(
        fingerprint.trim_start_matches("fnv1a64:")
    ));
}
