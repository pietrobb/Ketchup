//! The drawing files each document wrote, with the SHA-256 of each, so a
//! client may replace its document's own file unasked (review 2026-11-04
//! MCP-140, r19 SEC-161) and no other.
//!
//! They are kept with the user's settings, not in the document (review
//! 2026-11-06 SEC-182): noting a file there made the document unsaved, the
//! note was lost with a save the user declined, and Save As or a copy of the
//! file carried it to another document. A saved document's files are kept by
//! the canonical path of its file, one `drawing=` line each: the document's
//! path, its id, the drawing's path and its SHA-256, a tab between. A
//! document moved or renamed, so that no file is left where it was, keeps
//! them by its id (r19 MF-162); a copy, Save As or a template's, leaves the
//! original where it was and starts with none. A document not saved keeps
//! its own in memory, and the file it is first saved as takes them over. A
//! drawing the system says is gone is forgotten at the next export.

use super::drawing_sheet_export::written_key;
use crate::recent_files::user_settings::SettingsFile;
use crate::*;

/// The key of a written drawing's line.
const DRAWING: &str = "drawing";

/// The settings file of the user that keeps the written drawings.
const FILE_NAME: &str = "drawings.txt";

/// The user's file of written drawings, when the platform names a folder for it.
pub(crate) fn of_user() -> Option<SettingsFile> {
    SettingsFile::of_user_named(FILE_NAME)
}

/// The system says nothing is at `path`; any other failure is no answer.
fn gone(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_err_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        )
    })
}

/// A path as one field of a line, none for a path that cannot be one.
fn field(path: &Path) -> Option<&str> {
    path.to_str()
        .filter(|text| !text.is_empty() && !text.contains(['\t', '\n', '\r']))
}

/// The saved document whose drawings are kept: its file's canonical path and its id.
struct Owner {
    file: PathBuf,
    /// The file as the field of a line.
    field: String,
    id: String,
}

impl Owner {
    /// Whether a line of `document`, of id `id`, is this document's: kept by
    /// its file, or by its id when no file is left where it was.
    fn holds(&self, document: &Path, id: &str) -> bool {
        document == self.file || id == self.id && gone(document)
    }
}

/// A `drawing=` line read.
struct Line<'a> {
    document: PathBuf,
    id: &'a str,
    drawing: PathBuf,
    sha256: &'a str,
}

fn parsed(line: &str) -> Option<Line<'_>> {
    let mut fields = line.split('\t');
    let mut next = || fields.next().filter(|text| !text.is_empty());
    let parsed = Line {
        document: PathBuf::from(next()?),
        id: next()?,
        drawing: PathBuf::from(next()?),
        sha256: next()?,
    };
    next().is_none().then_some(parsed)
}

impl KetchupApp {
    /// The saved document whose written drawings are kept with the user's
    /// settings; none while it is not saved or they are kept in memory only.
    fn written_owner(&self) -> Option<Owner> {
        self.window.written_drawings.as_ref()?;
        let path = self.file.path.as_deref()?;
        let file = match std::fs::canonicalize(path) {
            Ok(file) => file,
            Err(error) => {
                // Moved away while open: its drawings are kept in memory.
                ketchup_rejection::log!(
                    "{} does not resolve, so its written drawings are kept for this window only: {error}",
                    path.display()
                );
                return None;
            }
        };
        Some(Owner {
            field: field(&file)?.to_owned(),
            file,
            id: self.document.current().document_id().0.to_string(),
        })
    }

    /// The drawing files the document wrote: the SHA-256 of each, by
    /// [`written_key`].
    pub(crate) fn written_drawings(&self) -> BTreeMap<String, String> {
        let key =
            |path: &Path| ketchup_model::graph::sha256_hex(path.as_os_str().as_encoded_bytes());
        let mut written: BTreeMap<String, String> = self
            .drawings
            .written
            .iter()
            .map(|(path, sha256)| (key(path), sha256.clone()))
            .collect();
        if let (Some(store), Some(owner)) = (&self.window.written_drawings, self.written_owner()) {
            for line in store.values(DRAWING) {
                if let Some(line) = parsed(&line)
                    && owner.holds(&line.document, line.id)
                {
                    written.insert(key(&line.drawing), line.sha256.to_owned());
                }
            }
        }
        written
    }

    /// Notes the drawing files the window wrote for the document, each with
    /// its SHA-256, so they are its own after a restart too; the document
    /// stays as it was.
    pub(crate) fn wrote_drawings(&mut self, written: impl IntoIterator<Item = (PathBuf, String)>) {
        // Just written, the file is there; one moved away at once is not
        // this document's to replace.
        let written: Vec<(PathBuf, String)> = written
            .into_iter()
            .filter_map(|(path, sha256)| {
                std::fs::canonicalize(&path).ok().map(|path| (path, sha256))
            })
            .collect();
        if written.is_empty() {
            return;
        }
        let (Some(store), Some(owner)) = (&self.window.written_drawings, self.written_owner())
        else {
            self.drawings.written.extend(written);
            return;
        };
        let saved = store.update(DRAWING, |lines| {
            let mut kept: Vec<String> = lines
                .into_iter()
                .filter(|line| {
                    parsed(line).is_some_and(|line| {
                        !gone(&line.drawing)
                            && !(owner.holds(&line.document, line.id)
                                && written.iter().any(|(path, _)| *path == line.drawing))
                    })
                })
                .collect();
            kept.extend(written.iter().filter_map(|(path, sha256)| {
                field(path)
                    .map(|drawing| format!("{}\t{}\t{drawing}\t{sha256}", owner.field, owner.id))
            }));
            kept
        });
        if let Err(error) = saved {
            // Kept for this window at least; another asks the user again.
            ketchup_rejection::log!(
                "the drawing files the document wrote are not kept between windows: {error}"
            );
            self.drawings.written.extend(written);
        }
    }

    /// The document was saved, `before` the file it was saved as until now:
    /// the drawings written while it was not saved become its file's, and a
    /// document saved as another file starts with none (review 2026-11-06
    /// SEC-182).
    pub(super) fn drawings_saved(&mut self, before: Option<&Path>) {
        let now = self.file.path.as_deref();
        let same = |a: &Path, b: &Path| {
            a == b
                || std::fs::canonicalize(a)
                    .ok()
                    .zip(std::fs::canonicalize(b).ok())
                    .is_some_and(|(a, b)| a == b)
        };
        if before
            .zip(now)
            .is_some_and(|(before, now)| same(before, now))
        {
            return;
        }
        let unsaved = std::mem::take(&mut self.drawings.written);
        if before.is_none() {
            self.wrote_drawings(unsaved);
        }
    }

    /// Whether the file at `path`, of SHA-256 `sha256`, is the last drawing
    /// file the document wrote there, however the path is spelled. A path
    /// that does not read names no such file, so the client asks the user.
    pub(crate) fn wrote_drawing(&self, path: &Path, sha256: &str) -> bool {
        listed(&self.written_drawings(), path, sha256)
    }
}

/// Whether `written`, the drawing files a document wrote by [`written_key`],
/// has the file at `path` with SHA-256 `sha256`, however the path is spelled.
pub(crate) fn listed(written: &BTreeMap<String, String>, path: &Path, sha256: &str) -> bool {
    written_key(path).is_ok_and(|key| written.get(&key).is_some_and(|kept| kept == sha256))
}
