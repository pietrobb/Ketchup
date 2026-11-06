//! The drawing files each document wrote, with the SHA-256 of each, so a
//! client may replace its document's own file unasked (review 2026-11-04
//! MCP-140, r19 SEC-161) and no other.
//!
//! They are kept with the user's settings, not in the document (review
//! 2026-11-06 SEC-182): noting a file there made the document unsaved, the
//! note was lost with a save the user declined, and Save As or a copy of the
//! file carried it to another document. A saved document's files are kept by
//! the canonical path of its file, one `drawing=` line each: the document's
//! path, its file's token, the drawing's path and its SHA-256, a tab between.
//! A document moved or renamed, so that no file is left where it was, keeps
//! them by the token (r19 MF-162). The token is kept in the file and made
//! anew whenever the document is saved as another file, so a copy, Save As
//! or a template's, starts with none, whether or not the original is still
//! where it was (capability review run 529); the document id would not do,
//! as a copy shares it. A document not saved keeps its own in memory, and
//! the file it is first saved as takes them over. A drawing of the document
//! the system says is gone is forgotten on a worker after its next export.

use super::drawing_sheet_export::written_key;
use super::project_drawings::SHEET_NAMESPACE;
use crate::recent_files::presence::{Presence, presence};
use crate::recent_files::user_settings::SettingsFile;
use crate::*;

/// The key of a written drawing's line.
const DRAWING: &str = "drawing";

/// Where the document keeps the token of the file it was saved as.
const TOKEN_PATH: &str = "owner-v1.txt";

/// A line's token when its document had none: no moved file is it.
const NO_TOKEN: &str = "-";

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

/// Whether `a` and `b` name one file, however spelled.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(a, b)| a == b)
}

/// The saved document whose drawings are kept: its file's canonical path and
/// the token kept in the file.
struct Owner {
    file: PathBuf,
    /// The file as the field of a line.
    field: String,
    token: Option<String>,
}

impl Owner {
    /// Whether a line of `document`, of token `token`, is this document's:
    /// kept by its file, or by its token when no file is left where it was.
    fn holds(&self, document: &Path, token: &str) -> bool {
        document == self.file
            || self.token.as_deref().is_some_and(|own| own == token) && gone(document)
    }

    /// Whether a line of `document`, of token `token`, may be this document's,
    /// the disk not asked: kept by its file or by its token.
    fn kept_by(&self, document: &Path, token: &str) -> bool {
        document == self.file || self.token.as_deref().is_some_and(|own| own == token)
    }
}

/// A `drawing=` line read.
struct Line<'a> {
    document: PathBuf,
    token: &'a str,
    drawing: PathBuf,
    sha256: &'a str,
}

fn parsed(line: &str) -> Option<Line<'_>> {
    let mut fields = line.split('\t');
    let mut next = || fields.next().filter(|text| !text.is_empty());
    let parsed = Line {
        document: PathBuf::from(next()?),
        token: next()?,
        drawing: PathBuf::from(next()?),
        sha256: next()?,
    };
    next().is_none().then_some(parsed)
}

impl KetchupApp {
    /// The token of the file the document was saved as, kept in the file: it
    /// goes along when the file is moved, and not to a copy saved as another.
    fn file_token(&self) -> Option<&str> {
        self.file
            .container_data
            .extensions()
            .find(|entry| entry.namespace() == SHEET_NAMESPACE && entry.path() == TOKEN_PATH)
            .and_then(|entry| std::str::from_utf8(entry.bytes()).ok())
            .filter(|token| {
                !token.is_empty() && *token != NO_TOKEN && !token.contains(['\t', '\n', '\r'])
            })
    }

    /// Before the document is saved as `path`: saved as a file other than
    /// its own, or one with no token yet, it takes a new token, so a copy is
    /// never taken for the original moved (review 2026-11-06 SEC-182,
    /// capability review run 529).
    pub(super) fn token_for_save(&mut self, path: &Path) {
        let own = self.file.path.as_deref().or_else(|| {
            self.file
                .recovery_open
                .as_ref()
                .map(|recovery| recovery.requested_path.as_path())
        });
        if own.is_some_and(|own| same_file(own, path)) && self.file_token().is_some() {
            return;
        }
        let mut random = [0; 32];
        let token = match getrandom::fill(&mut random) {
            Ok(()) => ketchup_model::graph::sha256_hex(&random),
            Err(error) => {
                // No token: the file's drawings are its own where it is only.
                ketchup_rejection::log!(
                    "{} gets no token for its drawings: {error}",
                    path.display()
                );
                NO_TOKEN.to_owned()
            }
        };
        let entry = ketchup_model::persistence::ExtensionEntry::new(
            SHEET_NAMESPACE,
            TOKEN_PATH,
            false,
            token.into_bytes(),
        )
        .expect("the drawings namespace and the token's path are valid");
        self.file.container_data.set_extension(entry);
    }

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
            token: self.file_token().map(str::to_owned),
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
                    && owner.holds(&line.document, line.token)
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
        // Only this document's lines of the files written again are replaced:
        // the disk is not asked here, on the window's thread, and a line this
        // Kečup does not read, as a newer one's, is kept (review 2026-11-06 r23
        // SEC-202).
        let saved = store.update(DRAWING, |lines| {
            let mut kept: Vec<String> = lines
                .into_iter()
                .filter(|line| {
                    parsed(line).is_none_or(|line| {
                        !(owner.kept_by(&line.document, line.token)
                            && written.iter().any(|(path, _)| *path == line.drawing))
                    })
                })
                .collect();
            let token = owner.token.as_deref().unwrap_or(NO_TOKEN);
            kept.extend(written.iter().filter_map(|(path, sha256)| {
                field(path).map(|drawing| format!("{}\t{token}\t{drawing}\t{sha256}", owner.field))
            }));
            kept
        });
        if let Err(error) = saved {
            // Kept for this window at least; another asks the user again.
            ketchup_rejection::log!(
                "the drawing files the document wrote are not kept between windows: {error}"
            );
            self.drawings.written.extend(written);
            return;
        }
        if self
            .drawings
            .forgetting
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
        {
            let store = store.clone();
            self.drawings.forgetting =
                Some(std::thread::spawn(move || forget_gone(&store, &owner)));
        }
    }

    /// The document was saved, `before` the file it was saved as until now:
    /// the drawings written while it was not saved become its file's, and a
    /// document saved as another file starts with none (review 2026-11-06
    /// SEC-182).
    pub(super) fn drawings_saved(&mut self, before: Option<&Path>) {
        let now = self.file.path.as_deref();
        if before
            .zip(now)
            .is_some_and(|(before, now)| same_file(before, now))
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

/// Forgets the drawing files of `owner`'s document that the system says are
/// gone; one on a drive or share that does not answer is kept, as is every
/// other document's (review 2026-11-06 r23 SEC-202: each line of every
/// document was asked for on the window's thread, under the lock of the file,
/// and one offline was forgotten). Run on a worker: the disk is asked before
/// the lock is taken.
fn forget_gone(store: &SettingsFile, owner: &Owner) {
    let local = crate::recent_files::presence::local_folder();
    let gone: Vec<PathBuf> = store
        .values(DRAWING)
        .iter()
        .filter_map(|line| parsed(line))
        .filter(|line| owner.holds(&line.document, line.token))
        .map(|line| line.drawing)
        .filter(|drawing| presence(drawing, local.as_deref()) == Presence::Gone)
        .collect();
    if gone.is_empty() {
        return;
    }
    let forgotten = store.update(DRAWING, |lines| {
        lines
            .into_iter()
            .filter(|line| {
                parsed(line).is_none_or(|line| {
                    !(gone.contains(&line.drawing) && owner.holds(&line.document, line.token))
                })
            })
            .collect()
    });
    if let Err(error) = forgotten {
        ketchup_rejection::log!("the drawing files that are gone stay listed: {error}");
    }
}

/// Whether `written`, the drawing files a document wrote by [`written_key`],
/// has the file at `path` with SHA-256 `sha256`, however the path is spelled.
pub(crate) fn listed(written: &BTreeMap<String, String>, path: &Path, sha256: &str) -> bool {
    written_key(path).is_ok_and(|key| written.get(&key).is_some_and(|kept| kept == sha256))
}
