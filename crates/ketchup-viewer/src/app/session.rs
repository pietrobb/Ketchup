//! What a phone keeps of the Viewer so that it opens as it was after the
//! system ended the process in the background: the opened package, its
//! comments, and whether they were saved. The package is written once when it
//! is opened; a comment edit writes only the small comments file.

use super::ViewerApp;
use crate::review::Review;
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::PackageExport as _;
use std::io::Write as _;
use std::path::Path;

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

    /// Keep the opened package and its comments in `dir`, the app's private
    /// storage, when they changed since `last`; returns the mark now kept.
    /// Another package first drops the kept comments, so a process ended
    /// halfway never pairs them with it. The unsaved marker is written before
    /// the comments and removed after them, so unsaved comments never show as
    /// saved.
    pub fn keep_session(
        &self,
        dir: &Path,
        last: Option<SessionMark>,
    ) -> Result<Option<SessionMark>, Rejection> {
        let (Some(mark), Some(model), Some(review)) =
            (self.session_mark(), &self.model, &self.review)
        else {
            return Ok(last);
        };
        if Some(mark) == last {
            return Ok(last);
        }
        std::fs::create_dir_all(dir).map_err(|error| session_error(dir, error))?;
        let marker = dir.join(UNSAVED);
        let comments = dir.join(COMMENTS);
        if last.is_none_or(|last| last.package != mark.package) {
            for stale in [&comments, &marker] {
                remove_if_present(stale)?;
            }
            write_durably(&dir.join(PACKAGE), |writer| model.export().write(writer))?;
        }
        if mark.unsaved {
            std::fs::write(&marker, b"").map_err(|error| session_error(&marker, error))?;
        }
        write_durably(&comments, |writer| review.save(writer, model).map(drop))?;
        if !mark.unsaved {
            remove_if_present(&marker)?;
        }
        Ok(Some(mark))
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
        // them in the package.
        let comments = dir.join(COMMENTS);
        if comments.exists()
            && let Some(model) = &self.model
        {
            let file =
                std::fs::File::open(&comments).map_err(|error| session_error(&comments, error))?;
            self.review = Some(Review::open(std::io::BufReader::new(file), model)?);
        }
        if dir.join(UNSAVED).exists()
            && let Some(review) = &mut self.review
        {
            review.still_unsaved();
        }
        Ok(self.session_mark())
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
            std::fs::rename(&temporary, path).map_err(|error| session_error(path, error))
        });
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
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
