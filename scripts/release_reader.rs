//! Opens files with the persistence of a released Kečup (review 2026-10-29 proposal 5.4,
//! review 2026-10-31 CI-80).
//!
//! Built by `scripts/release_reads.py` against `ketchup-model` of the release's tag, not by
//! the workspace, and given what that release reads, taken from its source: the newest
//! format, its container schemas and its file size (0: none). Each
//! `saved-<format>-<name>.ketchup` in the directory given gets an empty document as its
//! `.recovery`, the way a file the release saved over has one. A file in a format and
//! container schema it reads, within its file size, must open editable (or for review
//! when it requires an extension, as in every version); any other must be refused, as
//! newer or as larger than the release opens. A file the release takes for damaged opens
//! its `.recovery` in its place, and its next Save would overwrite the newer file with
//! it: that fails the run. Given a fifth argument, a directory, it saves each file it
//! opens editable again there, for HEAD to find the same document in it (review
//! 2026-11-04 REL-143), and once more with its Undo history in `history/` there, as its
//! work recovery keeps it (review 2026-11-04 r19 P3).
use ketchup_model::document::DocumentStore;
use ketchup_model::persistence::{
    self, ContainerData, FileIdentity, FilePersistenceError, LoadNotice, LoadOutcome,
    PersistenceError,
};
use std::path::Path;
use std::process::ExitCode;

/// What the release reads.
struct Reads {
    newest_format: u16,
    container_schemas: Vec<u16>,
    max_bytes: Option<u64>,
}

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments
        .next()
        .expect("the directory of the files to open");
    let reads = Reads {
        newest_format: arguments
            .next()
            .and_then(|format| format.parse().ok())
            .expect("the newest format the release reads"),
        container_schemas: arguments
            .next()
            .expect("the container schemas the release reads")
            .split(',')
            .map(|schema| schema.parse().expect("a container schema"))
            .collect(),
        max_bytes: arguments
            .next()
            .and_then(|bytes| bytes.parse().ok())
            .filter(|bytes| *bytes > 0),
    };
    let resaved = arguments.next();
    let backup = persistence::save_document_store(&DocumentStore::new(), &ContainerData::default())
        .expect("an empty document saves");
    let mut files = std::fs::read_dir(&directory)
        .expect("the directory of the files to open")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "ketchup")
        })
        .collect::<Vec<_>>();
    files.sort();
    let mut failed = 0;
    for path in &files {
        let mut recovery = path.as_os_str().to_owned();
        recovery.push(".recovery");
        std::fs::write(&recovery, &backup).expect("write the .recovery");
        let name = path.file_name().unwrap().to_string_lossy();
        let opens = format_of(&name) <= reads.newest_format && fits(path, &reads);
        match verdict(path, opens, &reads).and_then(|verdict| match &resaved {
            Some(directory) if verdict == OPENED => save_again(path, directory).map(|()| verdict),
            _ => Ok(verdict),
        }) {
            Ok(verdict) => println!("ok      {name}: {verdict}"),
            Err(error) => {
                failed += 1;
                println!("FAILED  {name}: {error}");
            }
        }
    }
    println!("{} files, {failed} failed", files.len());
    if files.is_empty() || failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// The format a file named `saved-<format>-<name>.ketchup` is in.
fn format_of(name: &str) -> u16 {
    name.strip_prefix("saved-")
        .and_then(|rest| rest.get(..3))
        .and_then(|format| format.parse().ok())
        .unwrap_or_else(|| panic!("{name} does not name its format"))
}

fn larger(path: &Path, reads: &Reads) -> bool {
    reads
        .max_bytes
        .is_some_and(|max| std::fs::metadata(path).is_ok_and(|file| file.len() > max))
}

/// Whether the release reads the file `path` as it reads one it wrote: a container
/// schema it knows, within its file size.
fn fits(path: &Path, reads: &Reads) -> bool {
    let bytes = std::fs::read(path).expect("read a file to open");
    let schema = bytes
        .get(10..12)
        .map(|schema| u16::from_le_bytes([schema[0], schema[1]]));
    schema.is_some_and(|schema| reads.container_schemas.contains(&schema)) && !larger(path, reads)
}

const OPENED: &str = "opened";

/// Opens `path` and saves it under its name in `directory`, as a user who opens a file
/// and saves it does: what the release does not know is gone from that file, which HEAD
/// then compares with the original (review 2026-11-04 REL-143, `crates/ketchup-model/examples/resaved.rs`).
/// It saves it with its Undo history too, under its name in `directory/history`, as the
/// release's work recovery does: there an earlier revision can lose a field the current
/// one does not have (review 2026-11-04 r19 P3).
fn save_again(path: &Path, directory: &str) -> Result<(), String> {
    let LoadOutcome::Editable {
        document,
        container_data,
        ..
    } = persistence::load_file(path).map_err(|error| error.to_string())?
    else {
        return Err("opened for review only when saving it again".to_owned());
    };
    let file = persistence::save_document_store_current_snapshot(&document, &container_data)
        .map_err(|error| format!("could not save it again: {error}"))?;
    let name = path.file_name().unwrap();
    std::fs::write(Path::new(directory).join(name), file)
        .map_err(|error| format!("could not write it saved again: {error}"))?;
    save_again_with_history(path, &Path::new(directory).join("history"))
}

/// `checkpoint` as a work-recovery checkpoint of a file holding `base`, in schema 1,
/// which every release reads (`crates/ketchup-model/examples/resaved.rs` writes the same).
fn work_recovery(base: &[u8], checkpoint: &[u8]) -> Vec<u8> {
    let identity = FileIdentity::from_bytes(base);
    let mut bytes = b"KETCHUPWRK".to_vec();
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&identity.byte_len().to_le_bytes());
    bytes.extend_from_slice(&identity.sha256());
    bytes.extend_from_slice(&(checkpoint.len() as u64).to_le_bytes());
    bytes.extend_from_slice(checkpoint);
    bytes
}

/// Opens `path` with its Undo history and saves it with that history under its name in
/// `directory`, as the release's work recovery does: there an earlier revision can lose
/// a field the current one does not have (review 2026-11-04 r19 P3). A file opens
/// without its history; a work-recovery checkpoint beside it restores one, so the file
/// is opened as one beside a copy of itself. A release that does not restore the
/// history refuses it, and nothing is written.
fn save_again_with_history(path: &Path, directory: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let file = directory.join(path.file_name().unwrap());
    let checkpoint = persistence::work_recovery_path(&file);
    std::fs::create_dir_all(directory)
        .and_then(|()| std::fs::write(&file, &bytes))
        .and_then(|()| std::fs::write(&checkpoint, work_recovery(&bytes, &bytes)))
        .map_err(|error| format!("could not write it to open with its history: {error}"))?;
    let opened = persistence::load_file(&file);
    std::fs::remove_file(&checkpoint).map_err(|error| error.to_string())?;
    let restored = opened.as_ref().is_ok_and(|outcome| {
        let notices = &outcome.audit().notices;
        notices
            .iter()
            .any(|notice| matches!(notice, LoadNotice::OpenedWorkRecovery { .. }))
            && !notices
                .iter()
                .any(|notice| matches!(notice, LoadNotice::HistoryDiscarded { .. }))
    });
    match opened {
        Ok(LoadOutcome::Editable {
            document,
            container_data,
            ..
        }) if restored => {
            let saved = persistence::save_document_store(&document, &container_data)
                .map_err(|error| format!("could not save it again with its history: {error}"))?;
            std::fs::write(&file, saved).map_err(|error| {
                format!("could not write it saved again with its history: {error}")
            })
        }
        _ => std::fs::remove_file(&file).map_err(|error| error.to_string()),
    }
}

fn verdict(path: &Path, opens: bool, reads: &Reads) -> Result<&'static str, String> {
    match persistence::load_file(path) {
        Ok(outcome) => {
            if outcome
                .audit()
                .notices
                .iter()
                .any(|notice| matches!(notice, LoadNotice::OpenedBackup { .. }))
            {
                let why = std::fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| persistence::load(&bytes).map_err(|error| error.to_string()))
                    .err()
                    .unwrap_or_default();
                Err(format!(
                    "took the file for damaged ({why}) and opened its .recovery in its place"
                ))
            } else if !opens {
                Err("opened a file it should refuse".to_owned())
            } else if !outcome.is_editable()
                && outcome.container_data().requires_unknown_extension()
                && outcome.audit().migration_losses.is_empty()
            {
                // Every version opens a file another program marked as needing what it
                // added for review only.
                Ok("opened for review: it requires an extension")
            } else if !outcome.is_editable() {
                let audit = outcome.audit();
                Err(format!(
                    "opened for migration review only: losses {:?}, overrides {:?}",
                    audit.migration_losses, audit.override_health
                ))
            } else {
                Ok(OPENED)
            }
        }
        Err(FilePersistenceError::Format(
            PersistenceError::UnsupportedSchema(_)
            | PersistenceError::UnsupportedContainerSchema(_)
            | PersistenceError::UnsupportedContainerEntry(_)
            | PersistenceError::UnsupportedHistorySchema(_)
            | PersistenceError::UnknownFields(_),
        )) if !opens => Ok("refused as newer"),
        Err(FilePersistenceError::Format(PersistenceError::ResourceLimit))
            if !opens && larger(path, reads) =>
        {
            Ok("refused as larger than it opens")
        }
        Err(error) => Err(error.to_string()),
    }
}
