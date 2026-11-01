//! Every file Save writes, kept for the released Kečup versions to open (review
//! 2026-10-29 proposal 5.4, 2026-10-31 CI-80). With the feature `saved-files` and
//! `KETCHUP_SAVED_FILES_DIR` set, each saved file also goes there as
//! `saved-<format>-<sha256>.ketchup`, and `scripts/release_reads.py` opens them all with
//! the persistence of each released tag: what the tests save, not a list of documents
//! picked to stand for it. A check relaxed since a release changes no enum, so only a
//! file of each kind of document shows it (review 2026-10-26 MD-41).
use super::{MAGIC, container_directory};
use std::io::Write;
use std::path::Path;

/// Writes `file` to `KETCHUP_SAVED_FILES_DIR` if set.
pub(super) fn keep(file: &[u8]) {
    let Some(directory) = std::env::var_os("KETCHUP_SAVED_FILES_DIR") else {
        return;
    };
    let entries = container_directory(file).expect("a saved file has a container");
    let document = entries
        .iter()
        .find(|entry| entry.path == "document.bin")
        .expect("a saved file has a document.bin")
        .content;
    let format = u16::from_le_bytes([document[MAGIC.len()], document[MAGIC.len() + 1]]);
    let name = format!("saved-{format}-{}.ketchup", crate::graph::sha256_hex(file));
    let path = Path::new(&directory).join(name);
    if path.exists() {
        return;
    }
    // Tests save at once: a file appears whole, under its own name.
    let mut part = tempfile::Builder::new()
        .suffix(".part")
        .tempfile_in(&directory)
        .expect("a file in KETCHUP_SAVED_FILES_DIR");
    part.write_all(file).expect("write a saved file");
    part.persist(&path).expect("name a saved file");
}
