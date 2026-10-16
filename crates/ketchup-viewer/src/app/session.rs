//! What a phone keeps of the Viewer so that it opens as it was after the
//! system ended the process in the background: the opened package, its
//! comments, and whether they were saved. The package is written once when it
//! is opened, on a thread of its own so a large one does not hold up the
//! screen (VW-10); a comment edit writes only the small comments file.

use super::ViewerApp;
use crate::model::Model;
use crate::review::Review;
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::PackageExport as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

/// The opened package, as exported.
const PACKAGE: &str = "session.ketchup-view";
/// Every comment on it.
const COMMENTS: &str = "session.ketchup-comments";
/// Present while the comments in the session are not saved to a file.
const UNSAVED: &str = "session.unsaved";

/// The opened package and its comments as they are now; another package, any
/// edit or a save gives another mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionMark {
    package: [u8; 32],
    revision: u64,
    unsaved: bool,
}

impl ViewerApp {
    /// `None` while nothing is open.
    pub fn session_mark(&self) -> Option<SessionMark> {
        let model = self.model.as_ref()?;
        let review = self.review.as_ref()?;
        Some(SessionMark {
            package: model.export().payload_digest().ok()?,
            revision: review.revision(),
            unsaved: review.unsaved(),
        })
    }

    /// Open the session kept in `dir`, if there is one; comments that were
    /// unsaved stay unsaved, so leaving them still warns.
    pub fn restore_session(&mut self, dir: &Path) -> Result<Option<SessionMark>, Rejection> {
        let path = dir.join(PACKAGE);
        if !path.exists() {
            return Ok(None);
        }
        let file = std::fs::File::open(&path).map_err(|error| session_error(&path, error))?;
        self.open(std::io::BufReader::new(file))?;
        // A session kept before the comments had a file of their own carries
        // them in the package. Comments that do not open on it (kept beside
        // another package, or cut short) count as none: the package still opens.
        let comments = dir.join(COMMENTS);
        if comments.exists()
            && let Some(model) = &self.model
            && let Ok(file) = std::fs::File::open(&comments)
            && let Ok(review) = Review::open(std::io::BufReader::new(file), model)
        {
            self.review = Some(review);
        }
        if dir.join(UNSAVED).exists()
            && let Some(review) = &mut self.review
        {
            review.still_unsaved();
        }
        Ok(self.session_mark())
    }
}

/// A package being written to the session.
struct Writing {
    mark: SessionMark,
    handle: JoinHandle<Result<(), Rejection>>,
}

/// Keeps the opened package and its comments in a directory, the app's
/// private storage, as they change.
pub struct SessionKeeper {
    dir: PathBuf,
    /// The package kept in `dir`; comments are kept only beside it.
    package: Option<[u8; 32]>,
    /// The comments kept beside it, as their revision and whether unsaved.
    comments: Option<(u64, bool)>,
    writing: Option<Writing>,
    /// What keeping failed for: not tried again every frame, but at the next
    /// change (review 2026-10-15 N-11).
    failed: Option<SessionMark>,
}

impl SessionKeeper {
    /// Keeps the session in `dir`, which holds what `restored` names.
    pub fn new(dir: PathBuf, restored: Option<SessionMark>) -> Self {
        Self {
            dir,
            package: restored.map(|mark| mark.package),
            comments: restored.map(|mark| (mark.revision, mark.unsaved)),
            writing: None,
            failed: None,
        }
    }

    /// Whether a package is being written; its comments follow once it is on
    /// disk, so the caller keeps calling [`Self::keep`] until then.
    pub fn writing(&self) -> bool {
        self.writing.is_some()
    }

    /// Keeps what changed in `app` since the last call. Another package first
    /// drops the kept comments, so a process ended halfway never pairs them
    /// with it, and is then written on a thread of its own; its comments wait
    /// until it is on disk. The unsaved marker is written before the comments
    /// and removed after them, so unsaved comments never show as saved. A
    /// failed comments write keeps the package, so the next edit writes only
    /// the comments again.
    pub fn keep(&mut self, app: &ViewerApp) -> Result<(), Rejection> {
        if let Some(writing) = self.writing.take() {
            if !writing.handle.is_finished() {
                self.writing = Some(writing);
                return Ok(());
            }
            let path = self.dir.join(PACKAGE);
            let written = writing.handle.join().unwrap_or_else(|_| {
                Err(session_error(
                    &path,
                    std::io::Error::other("the package writer ended"),
                ))
            });
            if let Err(error) = written {
                self.failed = Some(writing.mark);
                return Err(error);
            }
            self.package = Some(writing.mark.package);
        }
        let (Some(mark), Some(model), Some(review)) = (app.session_mark(), &app.model, &app.review)
        else {
            return Ok(());
        };
        if self.failed == Some(mark) {
            return Ok(());
        }
        self.failed = None;
        let kept = self.write(mark, model, review);
        if kept.is_err() {
            self.failed = Some(mark);
        }
        kept
    }

    fn write(
        &mut self,
        mark: SessionMark,
        model: &Model,
        review: &Review,
    ) -> Result<(), Rejection> {
        let dir = &self.dir;
        let marker = dir.join(UNSAVED);
        let comments_path = dir.join(COMMENTS);
        if self.package != Some(mark.package) {
            std::fs::create_dir_all(dir).map_err(|error| session_error(dir, error))?;
            self.package = None;
            self.comments = None;
            for stale in [&comments_path, &marker] {
                remove_if_present(stale)?;
            }
            let path = dir.join(PACKAGE);
            let package = model.shared_export();
            self.writing = Some(Writing {
                mark,
                handle: std::thread::spawn(move || {
                    write_durably(&path, |writer| package.write(writer))
                }),
            });
            return Ok(());
        }
        if self.comments == Some((mark.revision, mark.unsaved)) {
            return Ok(());
        }
        self.comments = None;
        if mark.unsaved {
            // On disk, name included, before the comments it marks.
            std::fs::File::create(&marker)
                .and_then(|file| file.sync_all())
                .and_then(|()| sync_directory(dir))
                .map_err(|error| session_error(&marker, error))?;
        }
        write_durably(&comments_path, |writer| {
            review.save(writer, model).map(drop)
        })?;
        if !mark.unsaved {
            remove_if_present(&marker)?;
            sync_directory(dir).map_err(|error| session_error(dir, error))?;
        }
        self.comments = Some((mark.revision, mark.unsaved));
        Ok(())
    }
}

/// Write `path` through a synced temporary file renamed over it, so a
/// process ended halfway keeps the previous file whole.
fn write_durably(
    path: &Path,
    write: impl FnOnce(&mut dyn std::io::Write) -> Result<(), Rejection>,
) -> Result<(), Rejection> {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".part");
    let temporary = path.with_file_name(name);
    let written = std::fs::File::create(&temporary)
        .map_err(|error| session_error(&temporary, error))
        .and_then(|mut file| {
            let mut writer = std::io::BufWriter::new(&mut file);
            write(&mut writer)?;
            writer
                .flush()
                .map_err(|error| session_error(&temporary, error))?;
            drop(writer);
            file.sync_all()
                .map_err(|error| session_error(&temporary, error))
        })
        .and_then(|()| {
            std::fs::rename(&temporary, path)
                .and_then(|()| sync_directory(path.parent().unwrap_or(Path::new("."))))
                .map_err(|error| session_error(path, error))
        });
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// A rename, creation or removal in `dir` survives a power loss only once the
/// directory itself is synced; elsewhere there is no such call.
fn sync_directory(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<(), Rejection> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(session_error(path, error))
        }
        _ => Ok(()),
    }
}

fn session_error(path: &Path, error: std::io::Error) -> Rejection {
    Rejection::new("viewer.session", RejectionPhase::Io)
        .target(path.display().to_string())
        .reason("The Viewer could not keep the opened model for the next start.")
        .fix_hint("Free some storage on the device; the open model and comments stay on screen.")
        .caused_by(error)
}
