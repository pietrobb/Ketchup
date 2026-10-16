//! Replacing a document the customer chose in the save dialog. A content
//! provider offers no rename, so a write cut off halfway ("wt" truncates
//! first) would leave neither the old file nor the new one (VW-11). The old
//! bytes are kept in private storage first and written back when the new
//! ones do not get through.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Why a replacement did not happen, and what the document holds now.
#[derive(Debug)]
pub(crate) enum ReplaceFailure {
    /// The old bytes could not be kept, so nothing was written: the document
    /// is as it was.
    Unchanged(io::Error),
    /// Writing failed and the old bytes were written back.
    Restored(io::Error),
    /// Writing failed and so did writing back; the old bytes stay at `backup`.
    Damaged {
        write: io::Error,
        restore: io::Error,
        backup: PathBuf,
    },
}

/// Replaces the document read by `original` and written by `open_write`
/// (which truncates) with `bytes`, keeping its old bytes at `backup` until
/// the new ones are written.
pub(crate) fn replace_keeping_original<W: Write>(
    mut original: impl Read,
    mut open_write: impl FnMut() -> io::Result<W>,
    backup: &Path,
    bytes: &[u8],
) -> Result<(), ReplaceFailure> {
    let kept = File::create(backup)
        .and_then(|mut file| io::copy(&mut original, &mut file).and_then(|_| file.sync_all()));
    if let Err(error) = kept {
        drop(std::fs::remove_file(backup));
        return Err(ReplaceFailure::Unchanged(error));
    }
    let written = open_write().and_then(|mut file| {
        file.write_all(bytes)?;
        file.flush()
    });
    let Err(write) = written else {
        drop(std::fs::remove_file(backup));
        return Ok(());
    };
    let restored = File::open(backup).and_then(|mut old| {
        let mut file = open_write()?;
        io::copy(&mut old, &mut file)?;
        file.flush()
    });
    match restored {
        Ok(()) => {
            drop(std::fs::remove_file(backup));
            Err(ReplaceFailure::Restored(write))
        }
        Err(restore) => Err(ReplaceFailure::Damaged {
            write,
            restore,
            backup: backup.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A document that takes only `capacity` bytes per write before failing,
    /// like a provider whose storage fills up halfway.
    struct Document {
        bytes: RefCell<Vec<u8>>,
        capacities: RefCell<Vec<usize>>,
    }

    struct Writer<'a> {
        document: &'a Document,
        capacity: usize,
    }

    impl Write for Writer<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.capacity == 0 {
                return Err(io::Error::new(io::ErrorKind::StorageFull, "full"));
            }
            let n = buf.len().min(self.capacity);
            self.capacity -= n;
            self.document
                .bytes
                .borrow_mut()
                .extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Document {
        fn new(bytes: &[u8], capacities: &[usize]) -> Self {
            Self {
                bytes: RefCell::new(bytes.to_vec()),
                capacities: RefCell::new(capacities.iter().rev().copied().collect()),
            }
        }

        /// Opens for writing with "wt": the old bytes are gone at once.
        fn open_write(&self) -> io::Result<Writer<'_>> {
            self.bytes.borrow_mut().clear();
            let capacity = self.capacities.borrow_mut().pop().unwrap_or(usize::MAX);
            Ok(Writer {
                document: self,
                capacity,
            })
        }

        fn replace(&self, backup: &Path, bytes: &[u8]) -> Result<(), ReplaceFailure> {
            let original = self.bytes.borrow().clone();
            replace_keeping_original(original.as_slice(), || self.open_write(), backup, bytes)
        }
    }

    fn backup(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ketchup-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn a_write_cut_off_halfway_leaves_the_original() {
        let backup = backup("cut");
        let original = b"the customer's earlier comments".to_vec();
        let document = Document::new(&original, &[5]);
        let failure = document.replace(&backup, &[7; 64]).unwrap_err();
        assert!(
            matches!(&failure, ReplaceFailure::Restored(error) if error.kind() == io::ErrorKind::StorageFull),
            "{failure:?}"
        );
        assert_eq!(*document.bytes.borrow(), original);
        assert!(!backup.exists());
    }

    #[test]
    fn a_complete_write_replaces_and_leaves_nothing_behind() {
        let backup = backup("ok");
        let document = Document::new(b"old", &[]);
        document.replace(&backup, b"new comments").unwrap();
        assert_eq!(*document.bytes.borrow(), b"new comments");
        assert!(!backup.exists());
    }

    #[test]
    fn a_failed_write_back_keeps_the_original_in_private_storage() {
        let backup = backup("damaged");
        let document = Document::new(b"the original", &[3, 4]);
        match document.replace(&backup, b"new comments").unwrap_err() {
            ReplaceFailure::Damaged {
                write,
                restore,
                backup: kept,
            } => {
                assert_eq!(write.kind(), io::ErrorKind::StorageFull);
                assert_eq!(restore.kind(), io::ErrorKind::StorageFull);
                assert_eq!(std::fs::read(&kept).unwrap(), b"the original");
                std::fs::remove_file(kept).unwrap();
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nothing_is_written_when_the_original_cannot_be_kept() {
        let backup = backup("missing-dir").join("no").join("backup");
        let document = Document::new(b"the original", &[]);
        let failure = document.replace(&backup, b"new").unwrap_err();
        assert!(
            matches!(&failure, ReplaceFailure::Unchanged(error) if error.kind() == io::ErrorKind::NotFound),
            "{failure:?}"
        );
        assert_eq!(*document.bytes.borrow(), b"the original");
    }
}
