//! Replacing a document the customer chose in the save dialog. A content
//! provider offers no rename, so a write cut off halfway ("wt" truncates
//! first) would leave neither the old file nor the new one (VW-11). The old
//! bytes are kept in private storage first and written back when the new
//! ones do not get through. Next to them a record names the document and the
//! length and checksum of the new bytes, so a write that Android cut off by
//! ending the process is undone at the next start (`recover_interrupted`).

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The extension of the record kept beside a backup.
const RECORD: &str = "target";

/// What the record beside `backup` says: the document and its new bytes.
#[derive(Debug, PartialEq)]
struct Record {
    target: String,
    length: u64,
    checksum: u64,
}

impl Record {
    fn of(target: &str, bytes: &[u8]) -> Self {
        let mut checksum = Checksum::default();
        checksum.add(bytes);
        Self {
            target: target.to_owned(),
            length: bytes.len() as u64,
            checksum: checksum.0,
        }
    }

    fn text(&self) -> String {
        format!("{}\n{:016x}\n{}", self.length, self.checksum, self.target)
    }

    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.splitn(3, '\n');
        let length = lines.next()?.parse().ok()?;
        let checksum = u64::from_str_radix(lines.next()?, 16).ok()?;
        let target = lines.next()?.to_owned();
        Some(Self {
            target,
            length,
            checksum,
        })
    }
}

/// FNV-1a over the bytes: enough to tell the complete new bytes from a
/// prefix or from bytes another app wrote, not a defence against anyone.
struct Checksum(u64);

impl Default for Checksum {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Checksum {
    fn add(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn record_path(backup: &Path) -> PathBuf {
    backup.with_extension(RECORD)
}

/// Removes the record before the backup: a record without its backup is
/// dropped at the next start, a backup without its record is never used.
fn forget(backup: &Path) {
    drop(std::fs::remove_file(record_path(backup)));
    drop(std::fs::remove_file(backup));
}

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

/// Replaces the document `target`, read by `original` and written by
/// `open_write` (which truncates), with `bytes`, keeping its old bytes at
/// `backup` with a record beside it until the new ones are written.
pub(crate) fn replace_keeping_original<W: Write>(
    mut original: impl Read,
    mut open_write: impl FnMut() -> io::Result<W>,
    target: &str,
    backup: &Path,
    bytes: &[u8],
) -> Result<(), ReplaceFailure> {
    let kept = File::create(backup)
        .and_then(|mut file| io::copy(&mut original, &mut file).and_then(|_| file.sync_all()))
        .and_then(|()| {
            let mut record = File::create(record_path(backup))?;
            record.write_all(Record::of(target, bytes).text().as_bytes())?;
            record.sync_all()
        })
        .and_then(|()| sync_directory(backup));
    if let Err(error) = kept {
        forget(backup);
        return Err(ReplaceFailure::Unchanged(error));
    }
    let written = open_write().and_then(|mut file| {
        file.write_all(bytes)?;
        file.flush()
    });
    let Err(write) = written else {
        forget(backup);
        return Ok(());
    };
    match restore(&mut open_write, backup) {
        Ok(()) => {
            forget(backup);
            Err(ReplaceFailure::Restored(write))
        }
        // The record stays: the next start tries again.
        Err(restore) => Err(ReplaceFailure::Damaged {
            write,
            restore,
            backup: backup.to_owned(),
        }),
    }
}

fn restore<W: Write>(
    open_write: &mut impl FnMut() -> io::Result<W>,
    backup: &Path,
) -> io::Result<()> {
    let mut old = File::open(backup)?;
    let mut file = open_write()?;
    io::copy(&mut old, &mut file)?;
    file.flush()
}

/// The backup and its record survive the process only once the directory
/// naming them is on disk too.
fn sync_directory(backup: &Path) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(directory) = backup.parent() {
        File::open(directory)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = backup;
    Ok(())
}

/// What became of a replacement the process did not finish.
#[derive(Debug)]
pub(crate) enum Interrupted {
    /// The document held a cut-off write and has its old bytes back.
    Restored { target: String },
    /// The document could not be read or put back; its old bytes stay at
    /// `backup` and the next start tries again.
    Kept {
        target: String,
        backup: PathBuf,
        error: io::Error,
    },
}

/// Finishes the replacements in `private` that a process ending halfway left
/// behind: a document holding all of its new bytes keeps them; one holding
/// fewer, which a write cut off leaves, gets its old bytes back; one holding
/// more was written by another app since and is left alone.
pub(crate) fn recover_interrupted<R: Read, W: Write>(
    private: &Path,
    mut open_read: impl FnMut(&str) -> io::Result<R>,
    mut open_write: impl FnMut(&str) -> io::Result<W>,
) -> Vec<Interrupted> {
    let Ok(entries) = std::fs::read_dir(private) else {
        return Vec::new();
    };
    let mut records: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == RECORD)
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("replacing-"))
        })
        .collect();
    records.sort();
    let mut outcomes = Vec::new();
    for record_file in records {
        let backup = record_file.with_extension("bak");
        let Some(record) = std::fs::read_to_string(&record_file)
            .ok()
            .and_then(|text| Record::parse(&text))
        else {
            // Cut off before it was complete: nothing was written yet.
            forget(&backup);
            continue;
        };
        if !backup.is_file() {
            drop(std::fs::remove_file(&record_file));
            continue;
        }
        let kept = |error: io::Error| Interrupted::Kept {
            target: record.target.clone(),
            backup: backup.clone(),
            error,
        };
        let held = open_read(&record.target).and_then(|mut document| {
            let mut checksum = Checksum::default();
            let mut length = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = document.read(&mut buffer)?;
                if read == 0 {
                    return Ok((length, checksum.0));
                }
                checksum.add(&buffer[..read]);
                length += read as u64;
            }
        });
        match held {
            Ok((length, checksum)) if length == record.length && checksum == record.checksum => {
                forget(&backup);
            }
            Ok((length, _)) if length < record.length => {
                match restore(&mut || open_write(&record.target), &backup) {
                    Ok(()) => {
                        forget(&backup);
                        outcomes.push(Interrupted::Restored {
                            target: record.target.clone(),
                        });
                    }
                    Err(error) => outcomes.push(kept(error)),
                }
            }
            // Longer than the save wrote: another app wrote it since.
            Ok(_) => forget(&backup),
            Err(error) => outcomes.push(kept(error)),
        }
    }
    outcomes
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
        /// The process ends once the next writer has written this many bytes.
        ends_after: std::cell::Cell<Option<usize>>,
    }

    struct Writer<'a> {
        document: &'a Document,
        capacity: usize,
        ends_after: Option<usize>,
    }

    impl Write for Writer<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if let Some(left) = self.ends_after {
                let n = buf.len().min(left);
                self.document
                    .bytes
                    .borrow_mut()
                    .extend_from_slice(&buf[..n]);
                self.ends_after = Some(left - n);
                if left == n {
                    panic!("Android ended the process");
                }
                return Ok(n);
            }
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
                ends_after: std::cell::Cell::new(None),
            }
        }

        /// Opens for writing with "wt": the old bytes are gone at once.
        fn open_write(&self) -> io::Result<Writer<'_>> {
            self.bytes.borrow_mut().clear();
            let capacity = self.capacities.borrow_mut().pop().unwrap_or(usize::MAX);
            Ok(Writer {
                document: self,
                capacity,
                ends_after: self.ends_after.take(),
            })
        }

        fn replace(&self, backup: &Path, bytes: &[u8]) -> Result<(), ReplaceFailure> {
            let original = self.bytes.borrow().clone();
            replace_keeping_original(
                original.as_slice(),
                || self.open_write(),
                TARGET,
                backup,
                bytes,
            )
        }

        /// A save the process does not live through: it ends once `written`
        /// of the new bytes are in the document.
        fn replace_ending_after(&self, backup: &Path, bytes: &[u8], written: usize) {
            self.ends_after.set(Some(written));
            let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(self.replace(backup, bytes));
            }));
            assert!(ended.is_err(), "the save ended the process");
        }

        fn recover(&self, private: &Path) -> Vec<Interrupted> {
            recover_interrupted(
                private,
                |target| {
                    assert_eq!(target, TARGET);
                    Ok(io::Cursor::new(self.bytes.borrow().clone()))
                },
                |target| {
                    assert_eq!(target, TARGET);
                    self.open_write()
                },
            )
        }
    }

    const TARGET: &str = "content://com.android.providers.downloads.documents/document/42";

    /// A private directory of its own, so recoveries of tests running at the
    /// same time do not meet, and the backup path the Viewer would use in it.
    fn backup(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ketchup-replace-{}-{name}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("replacing-1.bak")
    }

    fn left_in(backup: &Path) -> usize {
        std::fs::read_dir(backup.parent().unwrap()).unwrap().count()
    }

    /// Review 2026-10-15 status VW-11: Android may end the process in the
    /// middle of a save; "wt" has already cut the document, and the old bytes
    /// kept aside were never written back. The next start puts them back.
    #[test]
    fn a_save_the_process_did_not_live_through_is_undone_at_the_next_start() {
        let backup = backup("ended");
        let original = b"the customer's earlier comments".to_vec();
        let document = Document::new(&original, &[]);
        document.replace_ending_after(&backup, &[7; 64], 10);
        assert_eq!(document.bytes.borrow().len(), 10);

        let outcomes = document.recover(backup.parent().unwrap());
        assert!(
            matches!(outcomes.as_slice(), [Interrupted::Restored { target }] if target == TARGET),
            "{outcomes:?}"
        );
        assert_eq!(*document.bytes.borrow(), original);
        assert_eq!(left_in(&backup), 0);
        assert!(document.recover(backup.parent().unwrap()).is_empty());
    }

    /// A process that ended after the new bytes were all written, before the
    /// old ones were let go, keeps the new bytes.
    #[test]
    fn a_save_that_wrote_everything_before_the_process_ended_is_kept() {
        let backup = backup("written");
        let document = Document::new(b"old", &[]);
        document.replace_ending_after(&backup, b"new comments", 12);
        assert!(document.recover(backup.parent().unwrap()).is_empty());
        assert_eq!(*document.bytes.borrow(), b"new comments");
        assert_eq!(left_in(&backup), 0);
    }

    /// A document another app wrote since is not put back; one that cannot
    /// be read keeps its old bytes aside for the next start.
    #[test]
    fn a_document_written_since_or_unreadable_is_not_put_back() {
        let backup = backup("since");
        let document = Document::new(b"old", &[]);
        document.replace_ending_after(&backup, b"new comments", 4);
        *document.bytes.borrow_mut() = b"written by another app since".to_vec();
        assert!(document.recover(backup.parent().unwrap()).is_empty());
        assert_eq!(*document.bytes.borrow(), b"written by another app since");
        assert_eq!(left_in(&backup), 0);

        let document = Document::new(b"old", &[]);
        document.replace_ending_after(&backup, b"new comments", 4);
        let outcomes = recover_interrupted(
            backup.parent().unwrap(),
            |_| Err::<io::Cursor<Vec<u8>>, _>(io::Error::from(io::ErrorKind::PermissionDenied)),
            |_| document.open_write(),
        );
        assert!(
            matches!(
                outcomes.as_slice(),
                [Interrupted::Kept { target, backup: kept, error }]
                    if target == TARGET && *kept == backup && error.kind() == io::ErrorKind::PermissionDenied
            ),
            "{outcomes:?}"
        );
        assert_eq!(std::fs::read(&backup).unwrap(), b"old");
        assert_eq!(document.recover(backup.parent().unwrap()).len(), 1);
        assert_eq!(*document.bytes.borrow(), b"old");
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
