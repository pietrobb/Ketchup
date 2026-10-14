//! What a phone keeps of the Viewer so that it opens as it was after the
//! system ended the process in the background: the opened package with its
//! comments, and whether they were saved.

use super::ViewerApp;
use ketchup_rejection::{Rejection, RejectionPhase};
use std::io::Write as _;
use std::path::Path;

/// The opened package with every comment, as a commented copy.
const PACKAGE: &str = "session.ketchup-view";
/// Present while the comments in the session are not saved to a file.
const UNSAVED: &str = "session.unsaved";

/// The opened package's comments as they are now; any edit, another package
/// or a save gives another mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionMark {
    revision: u64,
    unsaved: bool,
}

impl ViewerApp {
    /// `None` while nothing is open.
    pub fn session_mark(&self) -> Option<SessionMark> {
        self.model.as_ref()?;
        let review = self.review.as_ref()?;
        Some(SessionMark {
            revision: review.revision(),
            unsaved: review.unsaved(),
        })
    }

    /// Keep the opened package and its comments in `dir`, the app's private
    /// storage, when they changed since `last`; returns the mark now kept.
    /// The unsaved marker is written before the package and removed after
    /// it, so a process ended halfway never shows unsaved comments as saved.
    pub fn keep_session(
        &self,
        dir: &Path,
        last: Option<SessionMark>,
    ) -> Result<Option<SessionMark>, Rejection> {
        let Some(mark) = self.session_mark() else {
            return Ok(last);
        };
        if Some(mark) == last {
            return Ok(last);
        }
        std::fs::create_dir_all(dir).map_err(|error| session_error(dir, error))?;
        let marker = dir.join(UNSAVED);
        if mark.unsaved {
            std::fs::write(&marker, b"").map_err(|error| session_error(&marker, error))?;
        }
        let path = dir.join(PACKAGE);
        let mut name = path.file_name().unwrap_or_default().to_owned();
        name.push(".part");
        let temporary = path.with_file_name(name);
        let written = std::fs::File::create(&temporary)
            .map_err(|error| session_error(&temporary, error))
            .and_then(|mut file| {
                let mut writer = std::io::BufWriter::new(&mut file);
                self.save_commented_copy(&mut writer)?;
                writer
                    .flush()
                    .map_err(|error| session_error(&temporary, error))?;
                drop(writer);
                file.sync_all()
                    .map_err(|error| session_error(&temporary, error))
            })
            .and_then(|()| {
                std::fs::rename(&temporary, &path).map_err(|error| session_error(&path, error))
            });
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        if !mark.unsaved && marker.exists() {
            std::fs::remove_file(&marker).map_err(|error| session_error(&marker, error))?;
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
        if dir.join(UNSAVED).exists()
            && let Some(review) = &mut self.review
        {
            review.still_unsaved();
        }
        Ok(self.session_mark())
    }
}

fn session_error(path: &Path, error: std::io::Error) -> Rejection {
    Rejection::new("viewer.session", RejectionPhase::Io)
        .target(path.display().to_string())
        .reason("The Viewer could not keep the opened model for the next start.")
        .fix_hint("Free some storage on the device; the open model and comments stay on screen.")
        .caused_by(error)
}
