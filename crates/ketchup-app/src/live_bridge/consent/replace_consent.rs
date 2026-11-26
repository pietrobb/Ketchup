//! The user's answer to a client replacing a file its export did not write,
//! asked in the window once the client's request is answered (Vratimov
//! VRAT-8b). Asked while the request waited, a user away from the window held
//! it until the client gave up after 15 minutes with `response_timeout`, which
//! said nothing of the question. The client now hears at once that the window
//! asks, and the user's answer holds for its next request for that file, as
//! long as the file is unchanged since.

use crate::KetchupApp;
use ketchup_rejection::Rejection;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How long the user's answer waits for the client to send its export again.
pub(in crate::live_bridge) const ANSWER_HOLDS: Duration = Duration::from_secs(10 * 60);

/// What a client asks the user to agree to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::live_bridge) enum Replacing {
    /// Replace the file at the path.
    File(PathBuf),
    /// Take over another project's set listed at `list`, replacing `files`.
    Set { list: PathBuf, files: Vec<PathBuf> },
}

impl Replacing {
    /// The file the question is about.
    fn path(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Set { list, .. } => list,
        }
    }
}

/// Where the user's agreement to a client's replace stands.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::live_bridge) enum Consent {
    /// The user agreed to replacing the file as it is now.
    Agreed,
    /// The window asks the user once this request is answered.
    Asking,
    /// The user did not agree, for this reason.
    Declined(String),
}

struct Asked {
    replacing: Replacing,
    /// The SHA-256 of the file asked about when it was asked; none for one
    /// that does not read.
    sha256: Option<String>,
}

struct Answer {
    asked: Asked,
    agreed: Result<(), Rejection>,
    at: Instant,
}

/// The questions clients left for the user, and the user's answers.
#[derive(Default)]
pub(crate) struct ReplaceAsks {
    waiting: Vec<Asked>,
    answers: Vec<Answer>,
}

fn file_sha256(path: &Path) -> Option<String> {
    crate::export_bundle::export_target_sha256(path)
        .ok()
        .flatten()
}

impl KetchupApp {
    /// The user's answer to `replacing`, given for the file as it is now and
    /// taken by this request; without one, the window asks once the request is
    /// answered.
    pub(in crate::live_bridge) fn client_replace_consent(
        &mut self,
        replacing: Replacing,
    ) -> Consent {
        let sha256 = file_sha256(replacing.path());
        let asks = &mut self.live.replace_asks;
        asks.answers
            .retain(|answer| answer.at.elapsed() < ANSWER_HOLDS);
        if let Some(at) = asks
            .answers
            .iter()
            .position(|answer| answer.asked.replacing == replacing)
        {
            let answer = asks.answers.remove(at);
            // A file changed since the user answered is asked about again.
            if answer.asked.sha256 == sha256 {
                return match answer.agreed {
                    Ok(()) => Consent::Agreed,
                    Err(declined) => Consent::Declined(declined.reason_text().to_owned()),
                };
            }
        }
        if !asks
            .waiting
            .iter()
            .any(|asked| asked.replacing == replacing)
        {
            asks.waiting.push(Asked { replacing, sha256 });
        }
        Consent::Asking
    }

    /// Asks the user the questions clients' requests left, after those
    /// requests were answered.
    pub(crate) fn ask_client_replace(&mut self) {
        for asked in std::mem::take(&mut self.live.replace_asks.waiting) {
            let agreed = match &asked.replacing {
                Replacing::File(path) => {
                    self.authorize_overwrite(path, path.to_string_lossy().as_bytes())
                }
                Replacing::Set { list, files } => {
                    let files: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
                    self.authorize_set_take_over(list, &files)
                }
            };
            self.live.replace_asks.answers.push(Answer {
                asked,
                agreed,
                at: Instant::now(),
            });
        }
    }
}
