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
//! it: that fails the run.
use ketchup_model::document::DocumentStore;
use ketchup_model::persistence::{
    self, ContainerData, FilePersistenceError, LoadNotice, PersistenceError,
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
    let directory = arguments.next().expect("the directory of the files to open");
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
        match verdict(path, opens, &reads) {
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
                Ok("opened")
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
