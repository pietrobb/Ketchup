//! Replacing a document the customer chose in the save dialog. A content
//! provider offers no rename, so a write cut off halfway ("wt" truncates
//! first) would leave neither the old file nor the new one (VW-11). The old
//! bytes and the new ones are kept in private storage first, and the old ones
//! are written back when the new ones do not get through. A record beside
//! them names the document and the length and checksum of both, so a write
//! that Android cut off by ending the process is undone at the next start
//! (`recover_interrupted`). Only a document holding part of the new or the
//! old bytes is put back; bytes another app wrote since stay, and the old
//! bytes are then set aside under a name of their own (review status R-3).

use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

/// The extension of the record kept beside a backup.
const RECORD: &str = "target";
/// The record while it is written; it gets [`RECORD`] once it is complete,
/// so a record cut off halfway never names a document.
const RECORD_PART: &str = "target-part";
/// The extension of the new bytes kept beside a backup.
const NEW: &str = "new";
/// The last line of a complete record.
const END: &str = "end";
/// The extension of the count of starts that could not finish a replacement.
const STARTS: &str = "starts";
/// After this many starts a replacement that cannot be finished is given up,
/// so the document's write grant is not kept for ever (review 2026-10-17 D-7).
const MAX_STARTS: u32 = 3;

/// The length and checksum of some bytes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Summary {
    length: u64,
    checksum: u64,
}

/// What the record beside a backup says: the document, its new bytes and
/// its old ones.
#[derive(Debug, PartialEq)]
struct Record {
    target: String,
    new: Summary,
    old: Summary,
}

impl Record {
    fn text(&self) -> String {
        format!(
            "{}\n{:016x}\n{}\n{:016x}\n{}\n{END}\n",
            self.new.length, self.new.checksum, self.old.length, self.old.checksum, self.target
        )
    }

    fn parse(text: &str) -> Option<Self> {
        let body = text.strip_suffix(&format!("\n{END}\n"))?;
        let mut lines = body.splitn(5, '\n');
        let new = summary(&mut lines)?;
        let old = summary(&mut lines)?;
        let target = lines.next().filter(|target| !target.is_empty())?;
        Some(Self {
            target: target.to_owned(),
            new,
            old,
        })
    }
}

fn summary<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Option<Summary> {
    Some(Summary {
        length: lines.next()?.parse().ok()?,
        checksum: u64::from_str_radix(lines.next()?, 16).ok()?,
    })
}

/// FNV-1a over the bytes and their count: enough to tell the bytes kept in
/// private storage from damaged ones, not a defence against anyone.
struct Checksum(Summary);

impl Default for Checksum {
    fn default() -> Self {
        Self(Summary {
            length: 0,
            checksum: 0xcbf2_9ce4_8422_2325,
        })
    }
}

impl Checksum {
    fn add(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0.checksum = (self.0.checksum ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
        self.0.length += bytes.len() as u64;
    }

    fn of(bytes: &[u8]) -> Summary {
        let mut checksum = Self::default();
        checksum.add(bytes);
        checksum.0
    }
}

/// A file written through, summing what it takes.
struct Summing {
    file: File,
    checksum: Checksum,
}

impl Write for Summing {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.file.write(buf)?;
        self.checksum.add(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Removes the record before the bytes: without a record nothing was
/// written to the document yet, so the bytes are dropped at the next start.
fn forget(backup: &Path) {
    for extension in [RECORD, RECORD_PART, NEW, STARTS] {
        drop(std::fs::remove_file(backup.with_extension(extension)));
    }
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
/// `backup`, the new ones and a record beside them until the new ones are
/// written.
pub(crate) fn replace_keeping_original<W: Write>(
    original: impl Read,
    mut open_write: impl FnMut() -> io::Result<W>,
    target: &str,
    backup: &Path,
    bytes: &[u8],
) -> Result<(), ReplaceFailure> {
    if let Err(error) = keep(original, target, backup, bytes) {
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

/// Keeps the old bytes at `backup` and the new ones beside them, then the
/// record naming both, each on disk before the next.
fn keep(mut original: impl Read, target: &str, backup: &Path, bytes: &[u8]) -> io::Result<()> {
    if target.is_empty() || target.contains('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the document's name cannot be recorded",
        ));
    }
    let mut old = Summing {
        file: File::create(backup)?,
        checksum: Checksum::default(),
    };
    io::copy(&mut original, &mut old)?;
    old.file.sync_all()?;
    let mut new = File::create(backup.with_extension(NEW))?;
    new.write_all(bytes)?;
    new.sync_all()?;
    let record = Record {
        target: target.to_owned(),
        new: Checksum::of(bytes),
        old: old.checksum.0,
    };
    let part = backup.with_extension(RECORD_PART);
    let mut file = File::create(&part)?;
    file.write_all(record.text().as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&part, backup.with_extension(RECORD))?;
    sync_directory(backup)
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

/// The kept bytes and their record survive the process only once the
/// directory naming them is on disk too.
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
    /// The document holds its old bytes: the save wrote nothing that stayed,
    /// and this start wrote nothing either.
    Unchanged { target: String },
    /// The document could not be read or put back; its old bytes stay at
    /// `backup` and the next start tries again.
    Kept {
        target: String,
        backup: PathBuf,
        error: io::Error,
    },
    /// [`MAX_STARTS`] starts could not finish the replacement: its old bytes
    /// are set aside at `kept` and it is no longer tried.
    GivenUp {
        target: String,
        kept: PathBuf,
        error: io::Error,
    },
    /// The document holds bytes the save did not write (another app wrote
    /// it since), or the record of the save cannot be read: the document is
    /// left as it is and its old bytes are kept at `kept`.
    Left {
        target: Option<String>,
        kept: PathBuf,
    },
}

/// Finishes the replacements in `private` that a process ending halfway left
/// behind: a document holding all of its new bytes keeps them; one holding
/// part of the new or the old bytes, which a cut-off write or write-back
/// leaves, gets its old bytes back; any other was written by another app
/// since and is left alone.
pub(crate) fn recover_interrupted<R: Read, W: Write>(
    private: &Path,
    mut open_read: impl FnMut(&str) -> io::Result<R>,
    mut open_write: impl FnMut(&str) -> io::Result<W>,
) -> Vec<Interrupted> {
    let Ok(entries) = std::fs::read_dir(private) else {
        return Vec::new();
    };
    let backups: std::collections::BTreeSet<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("replacing-"))
        })
        .map(|path| path.with_extension("bak"))
        .collect();
    let mut outcomes = Vec::new();
    for backup in backups {
        let text = match std::fs::read_to_string(backup.with_extension(RECORD)) {
            // The record was not complete: nothing was written yet.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                forget(&backup);
                continue;
            }
            text => text.ok(),
        };
        if !backup.is_file() {
            forget(&backup);
            continue;
        }
        let Some(record) = text.as_deref().and_then(Record::parse) else {
            outcomes.push(set_aside(&backup, None));
            continue;
        };
        outcomes.extend(recover(&record, &backup, &mut open_read, &mut open_write));
    }
    outcomes
}

/// The documents a replacement in `private` still has to finish at a start:
/// the only ones whose write grant is to be kept across starts.
pub(crate) fn pending_targets(private: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(private) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == RECORD)
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("replacing-"))
        })
        .filter_map(|path| Record::parse(&std::fs::read_to_string(path).ok()?))
        .map(|record| record.target)
        .collect()
}

fn recover<R: Read, W: Write>(
    record: &Record,
    backup: &Path,
    open_read: &mut impl FnMut(&str) -> io::Result<R>,
    open_write: &mut impl FnMut(&str) -> io::Result<W>,
) -> Option<Interrupted> {
    let kept = |error: io::Error| {
        let count = backup.with_extension(STARTS);
        let starts = std::fs::read_to_string(&count)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        if starts >= MAX_STARTS
            && let Some(kept) = put_aside(backup)
        {
            return Interrupted::GivenUp {
                target: record.target.clone(),
                kept,
                error,
            };
        }
        // Not counted when it cannot be written: the next start tries again.
        drop(std::fs::write(&count, starts.to_string()));
        Interrupted::Kept {
            target: record.target.clone(),
            backup: backup.to_owned(),
            error,
        }
    };
    let new = backup.with_extension(NEW);
    let held = check(&new, record.new)
        .and_then(|()| check(backup, record.old))
        .and_then(|()| open_read(&record.target))
        .and_then(|document| holds(document, &new, backup));
    let restored = || Interrupted::Restored {
        target: record.target.clone(),
    };
    match held {
        Ok((Prefix::Whole, _)) => {
            forget(backup);
            None
        }
        Ok((_, Prefix::Whole)) => {
            forget(backup);
            Some(Interrupted::Unchanged {
                target: record.target.clone(),
            })
        }
        Ok((Prefix::Part, _) | (_, Prefix::Part)) => {
            match restore(&mut || open_write(&record.target), backup) {
                Ok(()) => {
                    forget(backup);
                    Some(restored())
                }
                Err(error) => Some(kept(error)),
            }
        }
        Ok(_) => Some(set_aside(backup, Some(record.target.clone()))),
        Err(error) => Some(kept(error)),
    }
}

/// Moves the old bytes at `backup` out of the replacements, under a name
/// of their own, and drops the rest of the replacement.
fn set_aside(backup: &Path, target: Option<String>) -> Interrupted {
    // When it cannot be moved, the next start tries again.
    let kept = put_aside(backup).unwrap_or_else(|| backup.to_owned());
    Interrupted::Left { target, kept }
}

/// Moves the old bytes at `backup` to a name of their own and drops the
/// rest of the replacement; `None` when they cannot be moved.
fn put_aside(backup: &Path) -> Option<PathBuf> {
    let name = backup.file_name().map_or_else(Default::default, |name| {
        name.to_string_lossy().replacen("replacing-", "kept-", 1)
    });
    let kept = backup.with_file_name(name);
    std::fs::rename(backup, &kept).ok()?;
    forget(backup);
    Some(kept)
}

/// Fails unless the file at `path` still holds the bytes `summary` names.
fn check(path: &Path, summary: Summary) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut checksum = Checksum::default();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        checksum.add(&buffer[..read]);
    }
    if checksum.0 == summary {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the bytes kept in private storage no longer match their record",
        ))
    }
}

/// How the bytes of a document stand to some kept bytes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Prefix {
    Whole,
    Part,
    Not,
}

/// The kept bytes at a path, read alongside the document.
struct Alongside {
    file: BufReader<File>,
    matching: bool,
    expected: Vec<u8>,
}

impl Alongside {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: BufReader::new(File::open(path)?),
            matching: true,
            expected: Vec::new(),
        })
    }

    fn next(&mut self, chunk: &[u8]) -> io::Result<()> {
        if !self.matching {
            return Ok(());
        }
        self.expected.resize(chunk.len(), 0);
        match self.file.read_exact(&mut self.expected) {
            Ok(()) => self.matching = self.expected == chunk,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => self.matching = false,
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn end(mut self) -> io::Result<Prefix> {
        if !self.matching {
            return Ok(Prefix::Not);
        }
        Ok(if self.file.read(&mut [0u8])? == 0 {
            Prefix::Whole
        } else {
            Prefix::Part
        })
    }
}

/// How the `document` stands to the new bytes at `new` and the old ones at
/// `old`.
fn holds(mut document: impl Read, new: &Path, old: &Path) -> io::Result<(Prefix, Prefix)> {
    let (mut new, mut old) = (Alongside::open(new)?, Alongside::open(old)?);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = document.read(&mut buffer)?;
        if read == 0 {
            return Ok((new.end()?, old.end()?));
        }
        new.next(&buffer[..read])?;
        old.next(&buffer[..read])?;
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

    /// The files of replacements still in progress beside `backup`.
    fn left_in(backup: &Path) -> usize {
        std::fs::read_dir(backup.parent().unwrap())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("replacing-")
            })
            .count()
    }

    /// The document was left as it was and its old bytes, `original`, are
    /// set aside under a name of their own; the set-aside file is removed.
    fn assert_changed_and_kept(outcomes: &[Interrupted], original: &[u8]) {
        let [Interrupted::Left { target, kept }] = outcomes else {
            panic!("{outcomes:?}");
        };
        assert_eq!(target.as_deref(), Some(TARGET));
        assert_set_aside(kept, original);
    }

    fn assert_set_aside(kept: &Path, original: &[u8]) {
        assert!(
            kept.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("kept-")),
            "{kept:?}"
        );
        assert_eq!(std::fs::read(kept).unwrap(), original);
        std::fs::remove_file(kept).unwrap();
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
        let outcomes = document.recover(backup.parent().unwrap());
        assert_eq!(*document.bytes.borrow(), b"written by another app since");
        assert_changed_and_kept(&outcomes, b"old");
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

    /// Review 2026-10-16 status R-3: shorter bytes another app wrote since
    /// are neither the new bytes cut off nor the old ones, so they stay.
    #[test]
    fn shorter_bytes_another_app_wrote_since_are_not_overwritten() {
        let backup = backup("shorter");
        let original = b"the customer's earlier comments".to_vec();
        let document = Document::new(&original, &[]);
        document.replace_ending_after(&backup, &[7; 64], 10);
        *document.bytes.borrow_mut() = b"short note".to_vec();
        let outcomes = document.recover(backup.parent().unwrap());
        assert_eq!(*document.bytes.borrow(), b"short note");
        assert_changed_and_kept(&outcomes, &original);
        assert_eq!(left_in(&backup), 0);
    }

    /// R-3: a write-back the process did not live through either leaves a
    /// prefix of the old bytes, longer than the new ones; it is put back.
    #[test]
    fn a_write_back_cut_off_too_is_finished_at_the_next_start() {
        let backup = backup("restore-cut");
        let original = b"the customer's earlier comments".to_vec();
        let document = Document::new(&original, &[]);
        document.replace_ending_after(&backup, b"short", 2);
        document.ends_after.set(Some(20));
        let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            document.recover(backup.parent().unwrap())
        }));
        assert!(ended.is_err(), "the write-back ended the process");
        assert_eq!(*document.bytes.borrow(), original[..20]);

        let outcomes = document.recover(backup.parent().unwrap());
        assert!(
            matches!(outcomes.as_slice(), [Interrupted::Restored { target }] if target == TARGET),
            "{outcomes:?}"
        );
        assert_eq!(*document.bytes.borrow(), original);
        assert_eq!(left_in(&backup), 0);
    }

    /// R-3: a record cut off while written, naming `…/4` for `…/42`, never
    /// names a document: under its own name until complete, nothing was
    /// written yet; one that does not end as a record does is not followed,
    /// and the old bytes are set aside.
    #[test]
    fn a_record_cut_off_while_written_names_no_document() {
        let backup = backup("torn");
        let torn = format!(
            "12\n{:016x}\n3\n{:016x}\n{}",
            1,
            2,
            &TARGET[..TARGET.len() - 1]
        );
        let document = Document::new(b"untouched", &[]);
        std::fs::write(&backup, b"old").unwrap();
        std::fs::write(backup.with_extension(RECORD_PART), &torn).unwrap();
        assert!(document.recover(backup.parent().unwrap()).is_empty());
        assert_eq!(left_in(&backup), 0);

        std::fs::write(&backup, b"old").unwrap();
        std::fs::write(backup.with_extension(RECORD), &torn).unwrap();
        let outcomes = document.recover(backup.parent().unwrap());
        let [Interrupted::Left { target: None, kept }] = outcomes.as_slice() else {
            panic!("{outcomes:?}");
        };
        assert_set_aside(kept, b"old");
        assert_eq!(*document.bytes.borrow(), b"untouched");
        assert_eq!(left_in(&backup), 0);
    }

    /// Review 2026-10-17 D-7: kept bytes that no longer match their record
    /// cannot be finished at any start; after three starts the old bytes are
    /// set aside with a report and the document's write grant is let go,
    /// instead of keeping it for ever.
    #[test]
    fn a_replacement_that_cannot_be_finished_is_given_up_after_three_starts() {
        let backup = backup("given-up");
        let document = Document::new(b"old", &[]);
        document.replace_ending_after(&backup, b"new comments", 4);
        std::fs::write(backup.with_extension(NEW), b"damaged").unwrap();
        let private = backup.parent().unwrap();
        for _ in 0..2 {
            let outcomes = document.recover(private);
            assert!(
                matches!(outcomes.as_slice(), [Interrupted::Kept { .. }]),
                "{outcomes:?}"
            );
            assert_eq!(pending_targets(private), [TARGET]);
        }
        let outcomes = document.recover(private);
        let [
            Interrupted::GivenUp {
                target,
                kept,
                error,
            },
        ] = outcomes.as_slice()
        else {
            panic!("{outcomes:?}");
        };
        assert_eq!(target, TARGET);
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_set_aside(kept, b"old");
        assert!(pending_targets(private).is_empty());
        assert_eq!(left_in(&backup), 0);
        assert!(document.recover(private).is_empty());
        assert_eq!(*document.bytes.borrow(), b"new ");
    }

    /// D-7: a save cut off before it wrote anything leaves the document as it
    /// was; the next start writes nothing and does not say it put it back.
    #[test]
    fn a_save_cut_off_before_it_wrote_anything_is_not_reported_as_put_back() {
        let backup = backup("nothing-written");
        let document = Document::new(b"old", &[]);
        keep(&b"old"[..], TARGET, &backup, b"new comments").unwrap();
        let outcomes = document.recover(backup.parent().unwrap());
        assert!(
            matches!(outcomes.as_slice(), [Interrupted::Unchanged { target }] if target == TARGET),
            "{outcomes:?}"
        );
        assert_eq!(*document.bytes.borrow(), b"old");
        assert_eq!(left_in(&backup), 0);
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
                // Its write grant is kept for the next start, and only its.
                let private = kept.parent().unwrap();
                assert_eq!(pending_targets(private), [TARGET]);
                assert!(document.recover(private).len() == 1);
                assert!(pending_targets(private).is_empty());
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
