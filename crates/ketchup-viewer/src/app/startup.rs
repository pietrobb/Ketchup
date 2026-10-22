//! What a phone opens when it starts: the session it kept, else the package
//! the development install placed. Either may be as large as a package may be
//! (`MAX_PACKAGE_FILE_BYTES`, about 268 MB); read and checked in the app's
//! first frame, on the UI thread, it held the screen long enough for Android
//! to close the app as not responding (review 2026-10-17 §3 P3). It is read on
//! a thread of its own, and the empty view says the model is loading.

use super::session::{KeptSession, SessionMark};
use super::{Prepared, ViewerApp};
use ketchup_rejection::Rejection;
use std::path::PathBuf;
use std::thread::JoinHandle;

/// The session and the package being read at the start.
pub struct StartupOpen {
    handle: JoinHandle<Started>,
    session_dir: Option<PathBuf>,
}

struct Started {
    session: Result<Option<KeptSession>, Rejection>,
    /// Read only when no session was.
    package: Option<Result<Prepared, Rejection>>,
}

impl StartupOpen {
    /// Starts reading the session kept in `session_dir`, else the package at
    /// `package` if it exists; `app` shows that the model is loading.
    pub fn start(
        app: &mut ViewerApp,
        session_dir: Option<PathBuf>,
        package: Option<PathBuf>,
    ) -> Self {
        app.set_loading(true);
        let kept_in = session_dir.clone();
        let handle = std::thread::spawn(move || {
            #[cfg(test)]
            super::tests::hold_startup_reader();
            let session = session_dir.as_deref().map_or(Ok(None), KeptSession::read);
            let package = match &session {
                Ok(Some(_)) => None,
                _ => package.filter(|path| path.exists()).map(|path| {
                    super::open_file(&path)
                        .and_then(|file| Prepared::read(std::io::BufReader::new(file)))
                }),
            };
            Started { session, package }
        });
        Self {
            handle,
            session_dir: kept_in,
        }
    }

    /// Whether [`Self::finish`] would not wait.
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Shows what was read in `app`, waiting for it if need be. Gives the
    /// mark of the session it opened, for the [`super::SessionKeeper`], and
    /// every failure met on the way, which the caller logs; a package that
    /// does not open leaves the Viewer empty. A session that does not open
    /// with unsaved comments is held, not overwritten (review 2026-10-21 VW-18).
    pub fn finish(self, app: &mut ViewerApp) -> (Option<SessionMark>, Vec<Rejection>) {
        app.set_loading(false);
        let mut failures = Vec::new();
        let started = self.handle.join().unwrap_or_else(|_| Started {
            session: Err(startup_error()),
            package: None,
        });
        let mark = match started.session.and_then(|kept| match kept {
            Some(kept) => app.show_session(kept),
            None => Ok(None),
        }) {
            Ok(mark) => mark,
            Err(error) => {
                failures.push(error);
                if let Some(dir) = &self.session_dir {
                    app.hold_unopened_session(dir);
                }
                None
            }
        };
        if mark.is_none()
            && let Some(Err(error)) = started
                .package
                .map(|package| package.and_then(|package| app.show_prepared(package)))
        {
            failures.push(error);
        }
        (mark, failures)
    }
}

fn startup_error() -> Rejection {
    Rejection::new("viewer.startup", ketchup_rejection::RejectionPhase::Io)
        .reason("Reading the model kept from the last start ended unexpectedly.")
        .fix_hint("Open the model again.")
}
