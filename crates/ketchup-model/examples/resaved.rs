//! Whether a released Kečup keeps what it saves over (review 2026-11-04 REL-143).
//!
//! `scripts/release_reads.py` has each release open every saved file and save it again,
//! as a user who opens a file and saves it does. A field that release does not know, read
//! without refusing the file, is dropped by that save. This opens each file of `RESAVED`
//! and its original of the same name in `ORIGINALS` as HEAD, saves each as HEAD's Save
//! does and opens it again, and fails when they differ: in the document, its program, the
//! name of its revision, its extensions, or a blob the original keeps. A blob only the
//! file saved again keeps is no loss: an older release keeps the source of an import a
//! later revision removed, which HEAD's Save leaves out.
//!
//! `cargo run -p ketchup-model --example resaved -- ORIGINALS RESAVED [RELEASE]`
use ketchup_model::document::{DocumentStore, RuleProgramSource};
use ketchup_model::persistence::{self, ContainerData, ExtensionEntry, LoadOutcome};
use std::path::Path;
use std::process::ExitCode;

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
        if let Err(error) = same_document(&original, path) {
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

fn same_document(original: &Path, resaved: &Path) -> Result<(), String> {
    let before = Saved::by_head(original).map_err(|error| format!("the original: {error}"))?;
    let after = Saved::by_head(resaved).map_err(|error| format!("saved again: {error}"))?;
    let lost_blobs = before
        .blobs
        .iter()
        .filter(|(hash, bytes)| after.blobs.get(*hash) != Some(*bytes))
        .count();
    let differs = [
        (before.document != after.document, "the document"),
        (before.program != after.program, "its program"),
        (
            before.checkpoint != after.checkpoint,
            "the name of its revision",
        ),
        (before.extensions != after.extensions, "its extensions"),
        (lost_blobs > 0, "a blob it keeps"),
    ]
    .into_iter()
    .filter_map(|(differs, what)| differs.then_some(what))
    .collect::<Vec<_>>();
    if differs.is_empty() {
        Ok(())
    } else {
        Err(format!("saving it again changed {}", differs.join(", ")))
    }
}

/// What HEAD keeps of a file: it opens the file, saves it as Save does, and opens that.
struct Saved {
    document: Vec<u8>,
    program: Option<RuleProgramSource>,
    checkpoint: Option<String>,
    extensions: Vec<ExtensionEntry>,
    blobs: persistence::Blobs,
}

impl Saved {
    fn by_head(path: &Path) -> Result<Self, String> {
        let (document, container_data) = editable(persistence::load_file(path))?;
        let file = persistence::save_document_store_current_snapshot(&document, &container_data)
            .map_err(|error| error.to_string())?;
        let (document, container_data) = editable(persistence::load(&file))?;
        let checkpoint = document
            .revision_history()
            .nth(document.history_cursor())
            .and_then(|revision| revision.checkpoint())
            .map(str::to_owned);
        Ok(Self {
            document: persistence::save(&document.current()),
            program: document.current_rule_program().cloned(),
            checkpoint,
            extensions: container_data.extensions().cloned().collect(),
            blobs: container_data.blobs().clone(),
        })
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
