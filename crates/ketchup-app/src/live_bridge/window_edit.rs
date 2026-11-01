//! A pattern the window confirms is the typed CAD operation an MCP client
//! would send: `apply_and_verify` on a bridge without a socket, so the window
//! gets the same planner, copy names, limits, exact check and one Undo step.
//! Confirming only queues it; the next frame starts it and later frames poll.
//! Edits confirmed while an earlier one runs wait their turn, and Esc cancels
//! the running one and drops those waiting.
use super::plugin_menu::rejection_text;
use super::window_request::{Polled, WindowRequest};
use super::*;
use crate::app::hole::HoleSpec;
use ketchup_interaction::Ray;

/// How an edit confirmed while an earlier one ran is picked again on the
/// document that one left, whose faces belong to the earlier edit.
pub(crate) enum Repick {
    Hole { ray: Ray, spec: HoleSpec },
}

/// The edit a confirmed preview hands to the bridge.
struct QueuedEdit {
    /// The document the edit was confirmed on.
    expected: Stamp,
    program: AssistantCadEditProgram,
    repick: Option<Repick>,
    /// The digest the window shows once the edit is published.
    committed_digest_key: &'static str,
}

#[derive(Default)]
pub(crate) struct WindowEdits {
    /// Confirmed and not started yet, oldest first: four quick holes are four.
    queued: VecDeque<QueuedEdit>,
    run: Option<WindowRequest<&'static str>>,
    /// The document the running edit started from.
    running_from: u64,
    /// Mutation epochs the window's own edits moved the document through with
    /// nothing else in between, oldest first. An edit confirmed at one of them
    /// applies to the document the last one left; any other change since it
    /// was confirmed makes it stale.
    chain: Vec<u64>,
}

impl WindowEdits {
    pub(crate) fn is_running(&self) -> bool {
        !self.queued.is_empty() || self.run.is_some()
    }

    /// Cancels the running edit and drops the waiting ones; false when there
    /// was none.
    pub(crate) fn cancel(&mut self) -> bool {
        let waiting = !self.queued.is_empty();
        self.queued.clear();
        if let Some(run) = &self.run {
            run.cancel();
        }
        waiting || self.run.is_some()
    }
}

#[cfg(test)]
impl KetchupApp {
    /// Frames until the window edits are answered; fails after a minute with
    /// the status line, instead of guessing how long the bridge needs.
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
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

impl KetchupApp {
    /// Queues `operation` as one CAD edit of the document as it is now. While
    /// an earlier window edit runs it waits for it and then applies to the
    /// document that edit left, picked again by `repick` when it names faces.
    pub(crate) fn queue_window_edit(
        &mut self,
        operation: AssistantCadEditOperation,
        committed_digest_key: &'static str,
        repick: Option<Repick>,
    ) {
        self.live.edits.queued.push_back(QueuedEdit {
            expected: self.live_bridge_stamp(),
            program: AssistantCadEditProgram {
                operations: vec![operation],
            },
            repick,
            committed_digest_key,
        });
        self.status_key = "status-exact-pending";
    }

    /// Starts the next queued window edit and drives a running one; taken out
    /// of the app meanwhile, so it is not a busy reason for its own publication.
    pub(crate) fn poll_window_edit(&mut self, context: &egui::Context) {
        if self.live.edits.run.is_none()
            && let Some(edit) = self.live.edits.queued.pop_front()
        {
            let QueuedEdit {
                expected,
                mut program,
                repick,
                committed_digest_key,
            } = edit;
            let current = self.live_bridge_stamp();
            let edits = &mut self.live.edits;
            let follows = edits.chain.last() == Some(&current.mutation_epoch)
                && edits.chain.contains(&expected.mutation_epoch);
            edits.running_from = current.mutation_epoch;
            if follows
                && current.mutation_epoch != expected.mutation_epoch
                && let Some(Repick::Hole { ray, spec }) = repick
            {
                let Some(operation) = self.hole_on_ray(ray, &spec) else {
                    self.digest = self.catalog.text("digest-hole-face-required");
                    self.finish_window_edit_turn();
                    return;
                };
                program.operations = vec![operation];
            }
            let request = ApplyAndVerifyRequest {
                expected: Some(if follows { current } else { expected }),
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
            Polled::Answered(key, Ok(_)) => {
                let edits = &mut self.live.edits;
                if edits.chain.last() != Some(&edits.running_from) {
                    edits.chain = vec![edits.running_from];
                }
                edits.chain.push(self.document.mutation_epoch());
                self.catalog.text(key)
            }
            Polled::Answered(_, Err(refusal)) if refusal["code"] == "request_cancelled" => {
                self.catalog.text("digest-cancelled")
            }
            Polled::Answered(_, Err(refusal)) => rejection_text(&refusal),
            Polled::Disconnected(_) => rejection_text(&rejection_result(
                "apply_and_verify_worker_disconnected",
                None,
            )),
        };
        self.finish_window_edit_turn();
    }

    /// After an edit answered: with none waiting, the window is ready again.
    fn finish_window_edit_turn(&mut self) {
        if self.live.edits.queued.is_empty() {
            self.live.edits.chain.clear();
            self.status_key = "status-ready";
        }
    }
}
