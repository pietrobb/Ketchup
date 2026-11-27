//! The user's answer to a client replacing a file its export did not write,
//! asked in the window once the client's request is answered (Vratimov
//! VRAT-8b). Asked while the request waited, a user away from the window held
//! it until the client gave up after 15 minutes with `response_timeout`, which
//! said nothing of the question. The client now hears at once that the window
//! asks, and the user's agreement holds for its requests for that file, as
//! long as the file is unchanged since and for [`ANSWER_HOLDS`] at most.

use crate::KetchupApp;
use ketchup_rejection::{Rejection, RejectionPhase};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How long the user's answer waits for the client to send its export again.
pub(in crate::live_bridge) const ANSWER_HOLDS: Duration = Duration::from_secs(10 * 60);

/// The code of the rejection telling a client's export that the window asks
/// the user whether to replace a file (VRAT-8c).
pub(crate) const AWAITING_USER: &str = "side_effect.awaiting_user";

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
    /// The user's answer to `replacing`, given for the file as it is now;
    /// without one, the window asks once the request is answered. An agreement
    /// holds until the file changes or [`ANSWER_HOLDS`] passes, so a request that
    /// also waits on the question of another file, and writes nothing, finds it
    /// again when it comes back (review 2026-11-27 R1127-OW-1: it was taken by
    /// the first request, and two files out of step were asked about in turn
    /// for ever). A refusal is told once.
    pub(in crate::live_bridge) fn client_replace_consent(
        &mut self,
        replacing: Replacing,
    ) -> Consent {
        let sha256 = file_sha256(replacing.path());
        let asks = &mut self.live.replace_asks;
        // A file changed since the user answered is asked about again.
        asks.answers.retain(|answer| {
            answer.at.elapsed() < ANSWER_HOLDS
                && (answer.asked.replacing != replacing || answer.asked.sha256 == sha256)
        });
        if let Some(at) = asks
            .answers
            .iter()
            .position(|answer| answer.asked.replacing == replacing)
        {
            if asks.answers[at].agreed.is_ok() {
                return Consent::Agreed;
            }
            if let Err(declined) = asks.answers.remove(at).agreed {
                return Consent::Declined(declined.reason_text().to_owned());
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

    /// The user's agreement to a client's export replacing the file at `path`,
    /// as a rejection when there is none yet: `side_effect.awaiting_user` while
    /// the window asks, once the request is answered, and `side_effect.declined`
    /// when the user did not agree (VRAT-8c: the export waited on the dialog
    /// until the client gave up after 15 minutes).
    pub(crate) fn client_overwrite_consent(&mut self, path: &Path) -> Result<(), Rejection> {
        match self.client_replace_consent(Replacing::File(path.to_path_buf())) {
            Consent::Agreed => Ok(()),
            Consent::Asking => {
                let minutes = ANSWER_HOLDS.as_secs() / 60;
                Err(Rejection::new(AWAITING_USER, RejectionPhase::Validation).reason(format!(
                    "{} is not replaced yet: the window now asks the user whether to, and nothing was written. Tell the user to answer in the Kečup window, then send the same export again within {minutes} minutes; their answer holds for the file as it is now.",
                    path.display()
                )))
            }
            Consent::Declined(reason) => Err(Rejection::new(
                "side_effect.declined",
                RejectionPhase::Validation,
            )
            .reason(format!(
                "{reason} {} was not replaced and nothing was written; export to another path, or ask the user again.",
                path.display()
            ))),
        }
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
            // The newest answer to a question is the one that holds.
            let answers = &mut self.live.replace_asks.answers;
            answers.retain(|answer| answer.asked.replacing != asked.replacing);
            answers.push(Answer {
                asked,
                agreed,
                at: Instant::now(),
            });
        }
    }
}
