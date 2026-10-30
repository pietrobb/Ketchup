//! Opens files with the persistence of Kečup 0.2.0 (review 2026-10-29 proposal 5.4).
//!
//! Built by `scripts/release_0_2_0_reads.py` against `ketchup-model` of the `v0.2.0` tag,
//! not by the workspace. Each `release-0.2.0-<format>-<name>.ketchup` in the directory
//! given gets an empty document as its `.recovery`, the way a file 0.2.0 saved over has
//! one. A format 103 file in container schema 1 within 64 MiB must open editable (or for
//! review when it requires an extension, as in every version); any other must be refused,
//! as newer or as larger than 0.2.0 opens. A file 0.2.0 takes for damaged opens its
//! `.recovery` in its place, and the next Save of 0.2.0 would overwrite the newer file
//! with it: that fails the run.
use ketchup_model::document::DocumentStore;
use ketchup_model::persistence::{
    self, ContainerData, FilePersistenceError, LoadNotice, PersistenceError,
};
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let directory = std::env::args_os()
        .nth(1)
        .expect("the directory of the files to open");
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
        match verdict(path, format_of(&name) == 103 && fits(path)) {
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

/// The format a file named `release-0.2.0-<format>-<name>.ketchup` is in.
fn format_of(name: &str) -> u16 {
    name.strip_prefix("release-0.2.0-")
        .and_then(|rest| rest.get(..3))
        .and_then(|format| format.parse().ok())
        .unwrap_or_else(|| panic!("{name} does not name its format"))
}

/// Whether 0.2.0 reads the file `path` as it reads one it wrote: container schema 1,
/// within its file size.
fn fits(path: &Path) -> bool {
    let bytes = std::fs::read(path).expect("read a file to open");
    bytes.get(10..12) == Some(&1u16.to_le_bytes()[..])
        && bytes.len() <= persistence::MAX_NATIVE_DOCUMENT_BYTES
}

fn verdict(path: &Path, opens: bool) -> Result<&'static str, String> {
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
            if !opens
                && std::fs::metadata(path).is_ok_and(|file| {
                    file.len() > persistence::MAX_NATIVE_DOCUMENT_BYTES as u64
                }) =>
        {
            Ok("refused as larger than it opens")
        }
        Err(error) => Err(error.to_string()),
    }
}
