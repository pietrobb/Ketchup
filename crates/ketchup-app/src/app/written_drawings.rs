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
//! the file it is first saved as takes them over. A drawing of the document,
//! or of a document deleted, the system says is gone is forgotten on a
//! worker after the document's next export.

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

/// The identity of the file at `path` on its volume, which a rename keeps and
/// a copy does not: its device and inode, or on Windows its volume's serial
/// number and its file index. None when the system does not tell: a drawing
/// is then the document's where its file is only.
fn file_id(path: &Path) -> Option<String> {
    match identity_of(path) {
        Ok(id) => Some(id),
        Err(error) => {
            ketchup_rejection::log!(
                "{} has no identity its written drawings are kept by: {error}",
                path.display()
            );
            None
        }
    }
}

fn identity_of(path: &Path) -> std::io::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path)?;
        Ok(format!("{:x}:{:x}", metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        let information = winapi_util::file::information(std::fs::File::open(path)?)?;
        Ok(format!(
            "{:x}:{:x}",
            information.volume_serial_number(),
            information.file_index()
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("{} names no file identity on this system", path.display()),
        ))
    }
}

/// The saved document whose drawings are kept: its file's canonical path,
/// the token kept in the file and the file's identity.
struct Owner {
    file: PathBuf,
    /// The file as the field of a line.
    field: String,
    token: Option<String>,
    /// [`file_id`] of the file.
    id: Option<String>,
}

impl Owner {
    /// Whether a line of token `token` is of this document's file: a file of
    /// none is so only where it is.
    fn same_token(&self, token: &str) -> bool {
        token == self.token.as_deref().unwrap_or(NO_TOKEN)
    }

    /// Whether `line` is this document's: of its token, and kept by its file
    /// or, when no file is left where it was, by the identity of the file
    /// moved. A new document saved where another was, or a copy of the file
    /// made in a file manager, which has its token, is not it (review
    /// 2026-11-06 r23 SEC-203, P3 `Owner::holds`).
    fn holds(&self, line: &Line<'_>) -> bool {
        self.kept_by(line) && (line.document == self.file || gone(&line.document))
    }

    /// Whether `line` may be this document's, the disk not asked.
    fn kept_by(&self, line: &Line<'_>) -> bool {
        self.same_token(line.token)
            && (line.document == self.file
                || line.token != NO_TOKEN && line.id.is_some() && line.id == self.id.as_deref())
    }

    /// `line` as this document's: its drawing and SHA-256 kept by its file.
    fn line(&self, drawing: &str, sha256: &str) -> String {
        let token = self.token.as_deref().unwrap_or(NO_TOKEN);
        let mut line = format!("{}\t{token}\t{drawing}\t{sha256}", self.field);
        if let Some(id) = &self.id {
            line.push('\t');
            line.push_str(id);
        }
        line
    }
}

/// A `drawing=` line read: the document's path, its file's token, the
/// drawing's path, its SHA-256 and the identity of the document's file, none
/// in a line of a Kečup that did not keep it.
struct Line<'a> {
    document: PathBuf,
    token: &'a str,
    drawing: PathBuf,
    sha256: &'a str,
    id: Option<&'a str>,
}

fn parsed(line: &str) -> Option<Line<'_>> {
    let mut fields = line.split('\t');
    let mut next = || fields.next().filter(|text| !text.is_empty());
    let parsed = Line {
        document: PathBuf::from(next()?),
        token: next()?,
        drawing: PathBuf::from(next()?),
        sha256: next()?,
        id: next(),
    };
    next().is_none().then_some(parsed)
}

/// The token and identity of a document's file as on disk while a save of it
/// has not succeeded: what its written drawings are kept by until then, as a
/// failed Save As leaves the file as it was (review 2026-11-06 r23 P3).
#[derive(Clone, Debug, Default)]
pub(crate) struct OnDisk {
    token: Option<String>,
    id: Option<String>,
}

/// The document's file as the disk named it when it was resolved for `path`
/// as of `identity`: its canonical path, none when it did not resolve, and
/// its [`file_id`].
#[derive(Debug)]
pub(crate) struct Resolved {
    path: PathBuf,
    identity: Option<ketchup_model::persistence::FileIdentity>,
    file: Option<PathBuf>,
    id: Option<String>,
}

impl Resolved {
    fn of(path: &Path, identity: Option<ketchup_model::persistence::FileIdentity>) -> Self {
        let file = match std::fs::canonicalize(path) {
            Ok(file) => Some(file),
            Err(error) => {
                // Moved away while open: its drawings are kept in memory.
                ketchup_rejection::log!(
                    "{} does not resolve, so its written drawings are kept for this window only: {error}",
                    path.display()
                );
                None
            }
        };
        let id = file.as_deref().and_then(file_id);
        Self {
            path: path.to_owned(),
            identity,
            file,
            id,
        }
    }
}

impl KetchupApp {
    /// The token of the file the document was saved as, kept in the file: it
    /// goes along when the file is moved, and not to a copy saved as another.
    /// While a save has not succeeded, the token of the file as on disk.
    pub(crate) fn file_token(&self) -> Option<&str> {
        match &self.drawings.on_disk {
            Some(on_disk) => on_disk.token.as_deref(),
            None => self.container_token(),
        }
    }

    fn container_token(&self) -> Option<&str> {
        self.file
            .container_data
            .extensions()
            .find(|entry| entry.namespace() == SHEET_NAMESPACE && entry.path() == TOKEN_PATH)
            .and_then(|entry| std::str::from_utf8(entry.bytes()).ok())
            .filter(|token| {
                !token.is_empty() && *token != NO_TOKEN && !token.contains(['\t', '\n', '\r'])
            })
    }

    fn set_token(&mut self, token: String) {
        let entry = ketchup_model::persistence::ExtensionEntry::new(
            SHEET_NAMESPACE,
            TOKEN_PATH,
            false,
            token.into_bytes(),
        )
        .expect("the drawings namespace and the token's path are valid");
        self.file.container_data.set_extension(entry);
    }

    /// Before the document is saved as `path`: saved as a file other than
    /// its own, or one with no token yet, it takes a new token, so a copy is
    /// never taken for the original moved (review 2026-11-06 SEC-182,
    /// capability review run 529). So does a file whose token another file
    /// has kept drawings by, a copy made in a file manager (r23 SEC-203). The
    /// drawings are kept by the token on disk until the save succeeds (r23
    /// P3).
    pub(super) fn token_for_save(&mut self, path: &Path) {
        let own = self.file.path.as_deref().or_else(|| {
            self.file
                .recovery_open
                .as_ref()
                .map(|recovery| recovery.requested_path.as_path())
        });
        let on_disk = self.drawings.on_disk.clone().unwrap_or_else(|| OnDisk {
            token: self.container_token().map(str::to_owned),
            id: self.file.path.as_deref().and_then(file_id),
        });
        let kept = own.is_some_and(|own| same_file(own, path))
            && on_disk
                .token
                .as_deref()
                .is_some_and(|token| !self.token_of_another_file(token, on_disk.id.as_deref()));
        if kept && let Some(token) = on_disk.token.clone() {
            // A Save As that failed left the token the file has on disk.
            self.set_token(token);
            self.drawings.on_disk = Some(on_disk);
            return;
        }
        self.drawings.on_disk = Some(on_disk);
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
        self.set_token(token);
    }

    /// Whether another file than the document's, of an identity other than
    /// `id`, kept drawings by `token`: the document's file is its copy. The
    /// disk is not asked.
    fn token_of_another_file(&self, token: &str, id: Option<&str>) -> bool {
        let (Some(store), Some(owner)) = (&self.window.written_drawings, self.written_owner())
        else {
            return false;
        };
        store.values(DRAWING).iter().any(|line| {
            parsed(line).is_some_and(|line| {
                line.token == token
                    && line.document != owner.file
                    && line.id.is_some_and(|other| Some(other) != id)
            })
        })
    }

    /// The saved document whose written drawings are kept with the user's
    /// settings; none while it is not saved or they are kept in memory only.
    /// Its file is resolved once per file opened or saved: the disk is not
    /// asked, nor the same failure logged, at each export (review 2026-11-07
    /// P3 `written_owner`).
    fn written_owner(&self) -> Option<Owner> {
        self.window.written_drawings.as_ref()?;
        let path = self.file.path.as_deref()?;
        let mut resolved = self.drawings.resolved.borrow_mut();
        if resolved
            .as_ref()
            .is_none_or(|resolved| resolved.path != path || resolved.identity != self.file.identity)
        {
            *resolved = Some(Resolved::of(path, self.file.identity));
        }
        let resolved = resolved.as_ref()?;
        let file = resolved.file.clone()?;
        let id = match &self.drawings.on_disk {
            Some(on_disk) => on_disk.id.clone(),
            None => resolved.id.clone(),
        };
        Some(Owner {
            field: field(&file)?.to_owned(),
            file,
            token: self.file_token().map(str::to_owned),
            id,
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
                    && owner.holds(&line)
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
                        !(owner.kept_by(&line)
                            && written.iter().any(|(path, _)| *path == line.drawing))
                    })
                })
                .collect();
            kept.extend(written.iter().filter_map(|(path, sha256)| {
                field(path).map(|drawing| owner.line(drawing, sha256))
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
        // The file saved is another, of an identity of its own.
        self.drawings.resolved.take();
        let on_disk = self.drawings.on_disk.take();
        let now = self.file.path.as_deref();
        if before
            .zip(now)
            .is_some_and(|(before, now)| same_file(before, now))
        {
            if let Some(on_disk) = on_disk {
                self.drawings_kept_by_saved_file(on_disk);
            }
            return;
        }
        let unsaved = std::mem::take(&mut self.drawings.written);
        if before.is_none() {
            self.wrote_drawings(unsaved);
        }
    }

    /// The document was saved over its own file, which had the token and
    /// identity `on_disk`: its drawings are kept by the token and identity
    /// the file has now, those kept by the file where it was before it was
    /// moved here as well, so it is still taken for itself moved again and a
    /// copy of it is not (review 2026-11-06 r23 SEC-203).
    fn drawings_kept_by_saved_file(&mut self, on_disk: OnDisk) {
        let (Some(store), Some(now)) = (self.window.written_drawings.clone(), self.written_owner())
        else {
            return;
        };
        let before = Owner {
            file: now.file.clone(),
            field: now.field.clone(),
            token: on_disk.token,
            id: on_disk.id,
        };
        if before.token == now.token && before.id == now.id {
            return;
        }
        // Those kept by its file here: the disk is not asked.
        if let Err(error) =
            store.update(DRAWING, |lines| kept_again(lines, &before, &now, |_| false))
        {
            ketchup_rejection::log!(
                "the drawing files the document wrote are not kept by the file saved: {error}"
            );
            return;
        }
        // Those kept by the file where it was: asked on a worker, before the
        // lock is taken.
        let moved: Vec<PathBuf> = store
            .values(DRAWING)
            .iter()
            .filter_map(|line| parsed(line))
            .filter(|line| line.document != before.file && before.kept_by(line))
            .map(|line| line.document)
            .collect();
        if moved.is_empty() {
            return;
        }
        self.drawings.settling = Some(std::thread::spawn(move || {
            let gone: Vec<PathBuf> = moved.into_iter().filter(|path| gone(path)).collect();
            if gone.is_empty() {
                return;
            }
            if let Err(error) = store.update(DRAWING, |lines| {
                kept_again(lines, &before, &now, |document| {
                    gone.iter().any(|path| path == document)
                })
            }) {
                ketchup_rejection::log!(
                    "the drawing files of the file moved are not kept by the file saved: {error}"
                );
            }
        }));
    }

    /// Whether the file at `path`, of SHA-256 `sha256`, is the last drawing
    /// file the document wrote there, however the path is spelled. A path
    /// that does not read names no such file, so the client asks the user.
    pub(crate) fn wrote_drawing(&self, path: &Path, sha256: &str) -> bool {
        listed(&self.written_drawings(), path, sha256)
    }
}

/// `lines` with those of `before`'s document kept by its file, or by the file
/// it was where that is `moved`, kept by `now` instead.
fn kept_again(
    lines: Vec<String>,
    before: &Owner,
    now: &Owner,
    moved: impl Fn(&Path) -> bool,
) -> Vec<String> {
    lines
        .into_iter()
        .map(|text| {
            let again = parsed(&text)
                .filter(|line| {
                    before.kept_by(line) && (line.document == before.file || moved(&line.document))
                })
                .and_then(|line| {
                    field(&line.drawing).map(|drawing| now.line(drawing, line.sha256))
                });
            again.unwrap_or(text)
        })
        .collect()
}

/// Forgets the drawing files of `owner`'s document, and of documents the
/// system says are gone, that the system says are gone; one on a drive or
/// share that does not answer is kept, as is every other document's (review
/// 2026-11-06 r23 SEC-202: each line of every
/// document was asked for on the window's thread, under the lock of the file,
/// and one offline was forgotten). Run on a worker: the disk is asked before
/// the lock is taken.
fn forget_gone(store: &SettingsFile, owner: &Owner) {
    let local = crate::recent_files::presence::local_folder();
    let is_gone = |path: &Path| presence(path, local.as_deref()) == Presence::Gone;
    let lines = store.values(DRAWING);
    let gone: Vec<(PathBuf, PathBuf)> = lines
        .iter()
        .filter_map(|line| parsed(line))
        .filter(|line| owner.holds(line) || is_gone(&line.document))
        .filter(|line| is_gone(&line.drawing))
        .map(|line| (line.document, line.drawing))
        .collect();
    forget(store, owner, &gone);
}

/// Forgets the lines of `gone`, each a document and its drawing found gone
/// before the lock was taken, whose drawing is still not there under it: one
/// another window wrote again meanwhile keeps its line (review 2026-11-07 r26
/// P3). Only those drawings are asked for, and as they were gone, their
/// folders answer.
fn forget(store: &SettingsFile, owner: &Owner, gone: &[(PathBuf, PathBuf)]) {
    if gone.is_empty() {
        return;
    }
    // A line of a document deleted goes with its drawing too: no document
    // forgot them otherwise (review 2026-11-07 P3 `drawings.txt`).
    let forgotten = store.update(DRAWING, |lines| {
        let back: Vec<&Path> = gone
            .iter()
            .map(|(_, drawing)| drawing.as_path())
            .filter(|drawing| std::fs::symlink_metadata(drawing).is_ok())
            .collect();
        lines
            .into_iter()
            .filter(|line| {
                parsed(line).is_none_or(|line| {
                    back.contains(&line.drawing.as_path())
                        || !gone.iter().any(|(document, drawing)| {
                            *drawing == line.drawing
                                && (owner.kept_by(&line) || *document == line.document)
                        })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Review 2026-11-07 r26 P3: the drawings gone were found before the lock
    /// was taken, so one another window wrote again meanwhile lost its line.
    #[test]
    fn a_drawing_written_again_before_the_lock_keeps_its_line() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsFile::at(directory.path().join(FILE_NAME));
        let document = directory.path().join("plan.ketchup");
        let owner = Owner {
            field: document.display().to_string(),
            file: document.clone(),
            token: Some("t".to_owned()),
            id: None,
        };
        let drawing = directory.path().join("plan.dxf");
        let line = owner.line(&drawing.display().to_string(), "abc");
        store.replace(DRAWING, std::slice::from_ref(&line)).unwrap();
        let gone = [(document, drawing.clone())];

        std::fs::write(&drawing, b"written again").unwrap();
        forget(&store, &owner, &gone);
        assert_eq!(store.values(DRAWING), [line]);

        std::fs::remove_file(&drawing).unwrap();
        forget(&store, &owner, &gone);
        assert!(store.values(DRAWING).is_empty());
    }
}
