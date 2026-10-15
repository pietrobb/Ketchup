//! A window action that is one MCP request on a bridge without a socket, so
//! the window gets the same checks, refusals and Undo steps as a client. The
//! UI thread only starts it and polls each frame; the work runs on the
//! bridge's worker threads.
use super::program_dry_run::QueuedReply;
use super::*;

/// A started window action and the answer it will give.
pub(crate) struct WindowRequest<A> {
    pub(crate) action: A,
    /// Boxed: a bridge is kilobytes, and the request is moved every frame.
    pub(super) runner: Box<LiveBridge>,
    receiver: mpsc::Receiver<Response>,
    /// Set by `cancel`; the job answers `request_cancelled` at its next poll.
    cancelled: Arc<AtomicBool>,
}

/// How a window action stands after one poll.
pub(crate) enum Polled<A> {
    Running(WindowRequest<A>),
    /// The answer: the result, or the refusal with its named code.
    Answered(A, Result<Value, Value>),
    /// The worker went away without answering.
    Disconnected(A),
}

impl<A> WindowRequest<A> {
    /// Starts `start` on a bridge without a socket, unless a job an MCP client
    /// started keeps the window busy; then the answer is that refusal.
    pub(super) fn start(
        app: &mut KetchupApp,
        context: &egui::Context,
        action: A,
        start: impl FnOnce(&mut LiveBridge, &mut KetchupApp, QueuedReply),
    ) -> Self {
        take_error_details();
        let (reply, receiver) = mpsc::sync_channel(1);
        let mut runner = Box::new(transport::local());
        let queued = QueuedReply {
            id: 0,
            reply,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let cancelled = Arc::clone(&queued.cancelled);
        let client_jobs = app
            .live
            .bridge
            .as_ref()
            .map(LiveBridge::busy_jobs)
            .unwrap_or_default();
        if let Err(code) = busy::reject_if_busy(&client_jobs) {
            LiveBridge::reply(app, 0, &queued.reply, Err(code));
        } else {
            start(&mut runner, app, queued);
        }
        context.request_repaint();
        Self {
            action,
            runner,
            receiver,
            cancelled,
        }
    }

    /// Cancels the action: it publishes nothing and answers `request_cancelled`.
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// A request that stays running until the returned sender answers or drops.
    #[cfg(test)]
    pub(crate) fn held(action: A) -> (Self, mpsc::SyncSender<Response>) {
        let (reply, receiver) = mpsc::sync_channel(1);
        let request = Self {
            action,
            runner: Box::new(transport::local()),
            receiver,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        (request, reply)
    }

    /// Drives the bridge jobs one step and takes the answer once it is there.
    /// The caller holds the request outside the app meanwhile, so it is not a
    /// busy reason for its own publication.
    pub(crate) fn poll(mut self, app: &mut KetchupApp, context: &egui::Context) -> Polled<A> {
        let busy = ui_busy(context);
        self.runner.poll_apply_and_verify_job(app, context, busy);
        self.runner.poll_program_check_job(app, context);
        match self.receiver.try_recv() {
            Ok(response) if response.ok => {
                Polled::Answered(self.action, Ok(response.result.unwrap_or_default()))
            }
            Ok(response) => {
                Polled::Answered(self.action, Err(response.result.unwrap_or(Value::Null)))
            }
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(Duration::from_millis(20));
                Polled::Running(self)
            }
            Err(mpsc::TryRecvError::Disconnected) => Polled::Disconnected(self.action),
        }
    }
}
