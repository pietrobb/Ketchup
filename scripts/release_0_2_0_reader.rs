//! Opens files with the persistence of Kečup 0.2.0 (review 2026-10-29 proposal 5.4).
//!
//! Built by `scripts/release_0_2_0_reads.py` against `ketchup-model` of the `v0.2.0` tag,
//! not by the workspace. Each `release-0.2.0-<format>-<name>.ketchup` in the directory
//! given gets an empty document as its `.recovery`, the way a file 0.2.0 saved over has
//! one. A format 103 file must open editable; a newer one must be refused as newer. A file
//! 0.2.0 takes for damaged opens its `.recovery` in its place, and the next Save of 0.2.0
//! would overwrite the newer file with it: that fails the run.
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
        match verdict(path, format_of(&name)) {
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

fn verdict(path: &Path, format: u16) -> Result<&'static str, String> {
    match persistence::load_file(path) {
        Ok(outcome) => {
            if outcome
                .audit()
                .notices
                .iter()
                .any(|notice| matches!(notice, LoadNotice::OpenedBackup { .. }))
            {
                Err("took the file for damaged and opened its .recovery in its place".to_owned())
            } else if format != 103 {
                Err(format!("opened a format {format} file"))
            } else if !outcome.is_editable() {
                Err("opened for migration review only".to_owned())
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
        )) if format > 103 => Ok("refused as newer"),
        Err(error) => Err(error.to_string()),
    }
}
