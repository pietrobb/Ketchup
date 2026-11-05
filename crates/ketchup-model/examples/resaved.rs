//! Whether a released Kečup keeps what it saves over (review 2026-11-04 REL-143).
//!
//! `scripts/release_reads.py` has each release open every saved file and save it again,
//! as a user who opens a file and saves it does, and once more with its Undo history,
//! as the release's work recovery keeps it (`history/` beside them). A field that
//! release does not know, read without refusing the file, is dropped by that save. This
//! opens each file of `RESAVED` and its original of the same name in `ORIGINALS` as
//! HEAD, saves each the same way and opens it again, and fails when they differ: in the
//! document, its program, the name of its revision, its extensions, a blob the original
//! keeps, or, saved with its history, any revision of it (review 2026-11-04 r19 P3: only
//! the current revision was compared). A blob only the file saved again keeps is no
//! loss: an older release keeps the source of an import a later revision removed, which
//! HEAD's Save leaves out.
//!
//! `cargo run -p ketchup-model --example resaved -- ORIGINALS RESAVED [RELEASE]`
use ketchup_model::document::{DocumentStore, RuleProgramSource};
use ketchup_model::persistence::{
    self, ContainerData, ExtensionEntry, FileIdentity, LoadNotice, LoadOutcome,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

/// Where the release puts each file it saved again with its Undo history.
const HISTORY: &str = "history";

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let originals = arguments.next().expect("the directory of the saved files");
    let resaved = arguments
        .next()
        .expect("the directory of the files saved again");
    let release = arguments.next().unwrap_or_else(|| "the release".to_owned());
    let mut files = std::fs::read_dir(&resaved)
        .expect("the directory of the files saved again")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "ketchup")
        })
        .collect::<Vec<_>>();
    files.sort();
    let mut failed = 0;
    for path in &files {
        let name = path.file_name().expect("a file name");
        let original = Path::new(&originals).join(name);
        let with_history = Path::new(&resaved).join(HISTORY).join(name);
        if let Err(error) = same_document(&original, path, &with_history) {
            failed += 1;
            println!("FAILED  {}: {error}", name.to_string_lossy());
        }
    }
    println!(
        "{} files saved again by {release}, {failed} lost what HEAD wrote",
        files.len()
    );
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn same_document(original: &Path, resaved: &Path, with_history: &Path) -> Result<(), String> {
    let mut differs = Vec::new();
    // A release that does not restore the history refuses it: nothing of it is lost.
    let history =
        with_history
            .is_file()
            .then_some((with_history, true, "saving it again with its history"));
    for (again, history, how) in [(resaved, false, "saving it again")]
        .into_iter()
        .chain(history)
    {
        let before =
            Saved::by_head(original, history).map_err(|error| format!("the original: {error}"))?;
        let after = Saved::by_head(again, history).map_err(|error| format!("{how}: {error}"))?;
        let changed = before.lost_in(&after);
        if !changed.is_empty() {
            differs.push(format!("{how} changed {}", changed.join(", ")));
        }
    }
    if differs.is_empty() {
        Ok(())
    } else {
        Err(differs.join("; "))
    }
}

/// `checkpoint` as a work-recovery checkpoint of a file holding `base`, in schema 1,
/// which every release reads: `KETCHUPWRK`, the schema, the length and SHA-256 of the
/// file it was made from, and the checkpoint's length and bytes. `scripts/release_reader.rs`
/// writes the same.
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

/// `bytes` opened with the Undo history they hold. A file opens without it; only a
/// work-recovery checkpoint beside the file restores it, so `bytes` are opened as one
/// beside a file of the same bytes.
fn with_history(bytes: &[u8]) -> Result<(DocumentStore, ContainerData), String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let file = directory.path().join("file.ketchup");
    std::fs::write(&file, bytes)
        .and_then(|()| {
            std::fs::write(
                persistence::work_recovery_path(&file),
                work_recovery(bytes, bytes),
            )
        })
        .map_err(|error| error.to_string())?;
    let outcome = persistence::load_file(&file).map_err(|error| error.to_string())?;
    let notices = &outcome.audit().notices;
    if let Some(LoadNotice::HistoryDiscarded { reason }) = notices
        .iter()
        .find(|notice| matches!(notice, LoadNotice::HistoryDiscarded { .. }))
    {
        return Err(format!("HEAD does not restore its history: {reason}"));
    }
    if !notices
        .iter()
        .any(|notice| matches!(notice, LoadNotice::OpenedWorkRecovery { .. }))
    {
        return Err("HEAD does not restore its history".to_owned());
    }
    editable(Ok::<_, String>(outcome))
}

/// What HEAD keeps of a revision.
#[derive(PartialEq)]
struct Kept {
    document: Vec<u8>,
    program: Option<RuleProgramSource>,
    checkpoint: Option<String>,
}

/// What HEAD keeps of a file: it opens the file, saves it as Save does (or with its
/// history, as work recovery does), and opens that.
struct Saved {
    /// The current revision, or with the history each revision and the current one's place.
    revisions: Vec<Kept>,
    cursor: usize,
    /// By namespace and path, so the order they are listed in is no difference.
    extensions: BTreeMap<(String, String), ExtensionEntry>,
    blobs: persistence::Blobs,
}

impl Saved {
    fn by_head(path: &Path, history: bool) -> Result<Self, String> {
        let (document, container_data) = if history {
            with_history(&std::fs::read(path).map_err(|error| error.to_string())?)?
        } else {
            editable(persistence::load_file(path))?
        };
        let file = if history {
            persistence::save_document_store(&document, &container_data)
        } else {
            persistence::save_document_store_current_snapshot(&document, &container_data)
        }
        .map_err(|error| error.to_string())?;
        let (document, container_data) = if history {
            with_history(&file)?
        } else {
            editable(persistence::load(&file))?
        };
        let checkpoint = |index: usize| {
            document
                .revision_history()
                .nth(index)
                .and_then(|revision| revision.checkpoint())
                .map(str::to_owned)
        };
        let (revisions, cursor) = if history {
            let revisions = document
                .revision_history()
                .enumerate()
                .map(|(index, revision)| Kept {
                    document: persistence::save(revision.snapshot()),
                    program: revision.rule_program().cloned(),
                    checkpoint: checkpoint(index),
                })
                .collect();
            (revisions, document.history_cursor())
        } else {
            let current = Kept {
                document: persistence::save(&document.current()),
                program: document.current_rule_program().cloned(),
                checkpoint: checkpoint(document.history_cursor()),
            };
            (vec![current], 0)
        };
        Ok(Self {
            revisions,
            cursor,
            extensions: container_data
                .extensions()
                .map(|entry| {
                    (
                        (entry.namespace().to_owned(), entry.path().to_owned()),
                        entry.clone(),
                    )
                })
                .collect(),
            blobs: container_data.blobs().clone(),
        })
    }

    /// What `after` does not keep of `self`.
    fn lost_in(&self, after: &Self) -> Vec<&'static str> {
        let lost_blobs = self
            .blobs
            .iter()
            .filter(|(hash, bytes)| after.blobs.get(*hash) != Some(*bytes))
            .count();
        let (now, then) = (&self.revisions[self.cursor], &after.revisions[after.cursor]);
        let earlier = self.revisions.len() != after.revisions.len()
            || self.cursor != after.cursor
            || self.revisions != after.revisions;
        [
            (now.document != then.document, "the document"),
            (now.program != then.program, "its program"),
            (
                now.checkpoint != then.checkpoint,
                "the name of its revision",
            ),
            (earlier, "its Undo history"),
            (self.extensions != after.extensions, "its extensions"),
            (lost_blobs > 0, "a blob it keeps"),
        ]
        .into_iter()
        .filter_map(|(differs, what)| differs.then_some(what))
        .collect()
    }
}

fn editable<E: std::fmt::Display>(
    loaded: Result<LoadOutcome, E>,
) -> Result<(DocumentStore, ContainerData), String> {
    match loaded.map_err(|error| error.to_string())? {
        LoadOutcome::Editable {
            document,
            container_data,
            ..
        } => Ok((document, container_data)),
        LoadOutcome::ReviewOnly(_) => Err("HEAD opens it for review only".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::document::{CanonicalCommand, CommandBatch, DefinitionId};

    fn definition(id: u64, name: &str) -> CanonicalCommand {
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(id),
            name: name.into(),
        }
    }

    /// One empty document, as each test file opens it: its id is the same in each.
    fn empty() -> Vec<u8> {
        persistence::save_document_store(&DocumentStore::new(), &ContainerData::default()).unwrap()
    }

    /// The wall and the roof made one after the other in `order` from `empty`.
    fn document(empty: &[u8], order: [(u64, &str); 2]) -> DocumentStore {
        let (mut document, _) = editable(persistence::load(empty)).unwrap();
        for (id, name) in order {
            document
                .apply_batch(&CommandBatch::new(vec![definition(id, name)]))
                .unwrap();
        }
        document
    }

    const WALL_FIRST: [(u64, &str); 2] = [(1, "wall"), (2, "roof")];
    const ROOF_FIRST: [(u64, &str); 2] = [(2, "roof"), (1, "wall")];

    /// `original` saved with its history, and `resaved` as the release saves it again.
    fn compared(original: &DocumentStore, resaved: &DocumentStore) -> Result<(), String> {
        let directory = tempfile::tempdir().unwrap();
        let data = ContainerData::default();
        let original_path = directory.path().join("original.ketchup");
        let again = directory.path().join("again.ketchup");
        let with_history = directory.path().join("with-history.ketchup");
        let write = |path: &Path, bytes: Vec<u8>| std::fs::write(path, bytes).unwrap();
        write(
            &original_path,
            persistence::save_document_store(original, &data).unwrap(),
        );
        write(
            &again,
            persistence::save_document_store_current_snapshot(resaved, &data).unwrap(),
        );
        write(
            &with_history,
            persistence::save_document_store(resaved, &data).unwrap(),
        );
        same_document(&original_path, &again, &with_history)
    }

    #[test]
    fn a_file_saved_again_with_the_same_history_keeps_what_head_wrote() {
        let empty = empty();
        assert_eq!(
            compared(&document(&empty, WALL_FIRST), &document(&empty, WALL_FIRST)),
            Ok(())
        );
    }

    #[test]
    fn an_earlier_revision_the_release_saves_differently_is_found() {
        // Review 2026-11-04 r19 P3: the current revision alone was compared, so a release
        // that drops a field of an earlier revision when its work recovery saves the
        // history passed. The same document made in the other order differs only there.
        let empty = empty();
        let result = compared(&document(&empty, WALL_FIRST), &document(&empty, ROOF_FIRST));
        assert_eq!(
            result,
            Err("saving it again with its history changed its Undo history".to_owned())
        );
    }
}
