//! A pattern the window confirms is the typed CAD operation an MCP client
//! would send: `apply_and_verify` on a bridge without a socket, so the window
//! gets the same planner, copy names, limits, exact check and one Undo step.
//! Confirming only queues it; the next frame starts it and later frames poll.
use super::plugin_menu::rejection_text;
use super::window_request::{Polled, WindowRequest};
use super::*;

/// The edit a confirmed preview hands to the bridge.
struct QueuedEdit {
    expected: Stamp,
    program: AssistantCadEditProgram,
    /// The digest the window shows once the edit is published.
    committed_digest_key: &'static str,
}

#[derive(Default)]
pub(crate) struct WindowEdits {
    queued: Option<QueuedEdit>,
    run: Option<WindowRequest<&'static str>>,
}

impl WindowEdits {
    pub(crate) const fn is_running(&self) -> bool {
        self.queued.is_some() || self.run.is_some()
    }
}

#[cfg(test)]
impl KetchupApp {
    /// Frames until the window edit is answered; fails after a minute with the
    /// status line, instead of guessing how long the bridge needs.
    pub(crate) fn finish_window_edit(&mut self) {
        let context = egui::Context::default();
        let started = Instant::now();
        while self.live.edits.is_running() {
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the window edit was not delivered: {}",
                self.digest
            );
            self.poll_window_edit(&context);
            std::thread::yield_now();
        }
    }
}

impl KetchupApp {
    /// Queues `operation` as one CAD edit of the document as it is now; false
    /// while an earlier window edit has not answered yet.
    pub(crate) fn queue_window_edit(
        &mut self,
        operation: AssistantCadEditOperation,
        committed_digest_key: &'static str,
    ) -> bool {
        if self.live.edits.is_running() {
            return false;
        }
        self.live.edits.queued = Some(QueuedEdit {
            expected: self.live_bridge_stamp(),
            program: AssistantCadEditProgram {
                operations: vec![operation],
            },
            committed_digest_key,
        });
        self.status_key = "status-exact-pending";
        true
    }

    /// Starts a queued window edit and drives a running one; taken out of the
    /// app meanwhile, so it is not a busy reason for its own publication.
    pub(crate) fn poll_window_edit(&mut self, context: &egui::Context) {
        if let Some(edit) = self.live.edits.queued.take() {
            let QueuedEdit {
                expected,
                program,
                committed_digest_key,
            } = edit;
            let request = ApplyAndVerifyRequest {
                expected: Some(expected),
                selection: None,
                program,
                validators: Vec::new(),
                timeout_ms: DEFAULT_APPLY_VERIFY_TIMEOUT_MS,
                save: None,
                strict: false,
            };
            let run = WindowRequest::start(
                self,
                context,
                committed_digest_key,
                |runner, app, queued| {
                    runner.start_queued_apply_and_verify(
                        app,
                        context,
                        queued.id,
                        queued.reply,
                        queued.cancelled,
                        request,
                        false,
                    );
                },
            );
            self.live.edits.run = Some(run);
        }
        let Some(run) = self.live.edits.run.take() else {
            return;
        };
        self.digest = match run.poll(self, context) {
            Polled::Running(run) => {
                self.live.edits.run = Some(run);
                return;
            }
            Polled::Answered(key, Ok(_)) => self.catalog.text(key),
            Polled::Answered(_, Err(refusal)) => rejection_text(&refusal),
            Polled::Disconnected(_) => rejection_text(&rejection_result(
                "apply_and_verify_worker_disconnected",
                None,
            )),
        };
        self.status_key = "status-ready";
    }
}
