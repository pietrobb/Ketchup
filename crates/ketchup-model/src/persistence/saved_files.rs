//! Every file Save writes, kept for the released Kečup versions to open (review
//! 2026-10-29 proposal 5.4, 2026-10-31 CI-80). With the feature `saved-files` and
//! `KETCHUP_SAVED_FILES_DIR` set, each saved file also goes there as
//! `saved-<format>-<sha256>.ketchup`, and `scripts/release_reads.py` opens them all with
//! the persistence of each released tag: what the tests save, not a list of documents
//! picked to stand for it. A check relaxed since a release changes no enum, so only a
//! file of each kind of document shows it (review 2026-10-26 MD-41).
use super::{MAGIC, container_directory};
use std::io::{self, Write};
use std::path::Path;

/// Writes `file` to `KETCHUP_SAVED_FILES_DIR` if set. Keeping it is evidence for the
/// released readers, never a reason for Save to fail (review 2026-11-02 TST-122).
pub(super) fn keep(file: &[u8]) {
    if let Some(directory) = std::env::var_os("KETCHUP_SAVED_FILES_DIR") {
        keep_or_mark(Path::new(&directory), file);
    }
}

/// A file not kept leaves a `lost-*.lost` mark naming why, on which
/// `scripts/release_reads.py` fails: a log line alone is captured by libtest, and the
/// reads lost the file unseen (review 2026-11-03 TST-122). Save goes on either way.
fn keep_or_mark(directory: &Path, file: &[u8]) {
    let Err(error) = keep_in(directory, file) else {
        return;
    };
    let marked = tempfile::Builder::new()
        .prefix("lost-")
        .suffix(".lost")
        .tempfile_in(directory)
        .and_then(|mark| {
            let (mut written, _) = mark.keep().map_err(|kept| kept.error)?;
            write!(written, "{error}")
        });
    if let Err(mark_error) = marked {
        ketchup_rejection::log!(
            "saved-files: a saved file was not kept in {} ({error}), nor its mark ({mark_error})",
            directory.display()
        );
    }
}

fn keep_in(directory: &Path, file: &[u8]) -> io::Result<()> {
    let entries = container_directory(file).map_err(io::Error::other)?;
    let document = entries
        .iter()
        .find(|entry| entry.path == "document.bin")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no document.bin"))?
        .content;
    let format = u16::from_le_bytes([document[MAGIC.len()], document[MAGIC.len() + 1]]);
    let name = format!("saved-{format}-{}.ketchup", crate::graph::sha256_hex(file));
    let path = directory.join(name);
    if path.exists() {
        return Ok(());
    }
    // Tests save at once: a file appears whole, under its own name.
    let mut part = tempfile::Builder::new()
        .suffix(".part")
        .tempfile_in(directory)?;
    part.write_all(file)?;
    name_part(part, &path)
}

/// Gives `part` the name `path`. Two tests saving the same bytes race for one name;
/// the file already there holds those bytes, so it is kept, never replaced (on
/// Windows replacing it fails while the other test still has it open).
fn name_part(part: tempfile::NamedTempFile, path: &Path) -> io::Result<()> {
    match part.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(taken) if taken.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_another_test_holds_open_is_kept_as_it_is() {
        // The bytes differ so that a replacing rename fails this test on every platform,
        // not only on Windows, where replacing a file held open is refused (review
        // 2026-11-03 TST-122: on Linux the test passed without the fix).
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved-1-same.ketchup");
        std::fs::write(&path, b"first").unwrap();
        let _held = std::fs::File::open(&path).unwrap();
        let mut part = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        part.write_all(b"second").unwrap();
        name_part(part, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
    }

    #[test]
    fn a_file_that_cannot_be_kept_leaves_a_mark_instead_of_a_panic() {
        let directory = tempfile::tempdir().unwrap();
        keep_or_mark(directory.path(), b"not a container");
        let marks: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(marks.len(), 1, "{marks:?}");
        assert_eq!(marks[0].extension().unwrap(), "lost");
        assert!(!std::fs::read_to_string(&marks[0]).unwrap().is_empty());
    }

    #[test]
    fn a_directory_that_is_not_there_is_an_error_not_a_panic() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");
        let file = super::super::save_container(
            &crate::document::DocumentStore::new().current(),
            &super::super::ContainerData::default(),
        )
        .unwrap();
        assert!(keep_in(&missing, &file).is_err());
        keep_or_mark(&missing, &file);
    }
}
