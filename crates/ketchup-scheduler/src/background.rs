//! One background job with a deadline: it reports progress, can be cancelled,
//! and ends with exactly one result. The job's code sees one cancellation flag
//! that is set by `cancel`, by dropping the task, and when the deadline passes.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Why a job ended without its own result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskStop {
    Cancelled,
    TimedOut,
}

pub enum TaskEvent<P, R> {
    Progress(P),
    Finished(Result<R, TaskStop>),
}

/// What the job's code gets: the cancellation flag and a progress channel.
pub struct TaskContext<P, R> {
    cancelled: Arc<AtomicBool>,
    sender: Sender<TaskEvent<P, R>>,
}

impl<P, R> TaskContext<P, R> {
    /// The flag to hand to cancellable calls; set on cancel, drop or deadline.
    #[must_use]
    pub fn cancellation(&self) -> &AtomicBool {
        &self.cancelled
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Fails once nobody listens any more, so the job can stop early.
    pub fn progress(&self, progress: P) -> Result<(), TaskStop> {
        self.sender
            .send(TaskEvent::Progress(progress))
            .map_err(|_: mpsc::SendError<_>| TaskStop::Cancelled)
    }
}

pub struct BackgroundTask<P, R> {
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<TaskEvent<P, R>>,
    worker: Option<JoinHandle<()>>,
}

impl<P: Send + 'static, R: Send + 'static> BackgroundTask<P, R> {
    /// Runs `job` on a named thread. `finished` runs on that thread after the
    /// result is delivered, so a slow callback never delays `wait` or `poll`.
    pub fn spawn(
        name: &str,
        deadline: Duration,
        job: impl FnOnce(&TaskContext<P, R>) -> R + Send + 'static,
        finished: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        let started_at = Instant::now();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let context = TaskContext {
            cancelled: Arc::clone(&cancelled),
            sender,
        };
        let worker = std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let (value, late) =
                    run_until_deadline(&context.cancelled, started_at, deadline, || job(&context));
                let result = if late {
                    Err(TaskStop::TimedOut)
                } else if context.is_cancelled() {
                    Err(TaskStop::Cancelled)
                } else {
                    Ok(value)
                };
                if context.sender.send(TaskEvent::Finished(result)).is_ok() {
                    finished();
                }
            })?;
        Ok(Self {
            cancelled,
            receiver,
            worker: Some(worker),
        })
    }
}

/// Runs `job` while a watchdog sets `cancelled` when the deadline passes;
/// returns the job's value and whether the deadline passed.
fn run_until_deadline<T>(
    cancelled: &Arc<AtomicBool>,
    started_at: Instant,
    deadline: Duration,
    job: impl FnOnce() -> T,
) -> (T, bool) {
    let timed_out = Arc::new(AtomicBool::new(false));
    let (done, done_receiver) = mpsc::channel::<()>();
    let watchdog = {
        let cancelled = Arc::clone(cancelled);
        let timed_out = Arc::clone(&timed_out);
        std::thread::spawn(move || {
            let remaining = deadline.saturating_sub(started_at.elapsed());
            if let Err(RecvTimeoutError::Timeout) = done_receiver.recv_timeout(remaining) {
                timed_out.store(true, Ordering::Release);
                cancelled.store(true, Ordering::Release);
            }
        })
    };
    let value = job();
    drop(done);
    let _ = watchdog.join();
    let late = timed_out.load(Ordering::Acquire) || started_at.elapsed() >= deadline;
    (value, late)
}

impl<P, R> BackgroundTask<P, R> {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// The next event without blocking. A result that arrives after `cancel`
    /// reads as cancelled: the caller asked for no result.
    pub fn poll(&self) -> Result<TaskEvent<P, R>, TryRecvError> {
        self.receiver.try_recv().map(|event| self.settle(event))
    }

    /// The next event, blocking up to `timeout`.
    pub fn next_event(&self, timeout: Duration) -> Result<TaskEvent<P, R>, RecvTimeoutError> {
        self.receiver
            .recv_timeout(timeout)
            .map(|event| self.settle(event))
    }

    /// Blocks for the result, skipping progress; cancels the job and reports
    /// `TimedOut` when `timeout` passes first.
    pub fn wait(self, timeout: Duration) -> Result<R, TaskStop> {
        let started_at = Instant::now();
        loop {
            match self.next_event(timeout.saturating_sub(started_at.elapsed())) {
                Ok(TaskEvent::Progress(_)) => {}
                Ok(TaskEvent::Finished(result)) => return result,
                Err(RecvTimeoutError::Timeout) => {
                    self.cancel();
                    return Err(TaskStop::TimedOut);
                }
                Err(RecvTimeoutError::Disconnected) => return Err(TaskStop::Cancelled),
            }
        }
    }

    fn settle(&self, event: TaskEvent<P, R>) -> TaskEvent<P, R> {
        match event {
            TaskEvent::Finished(Ok(_)) if self.is_cancelled() => {
                TaskEvent::Finished(Err(TaskStop::Cancelled))
            }
            event => event,
        }
    }
}

impl<P, R> Drop for BackgroundTask<P, R> {
    /// Cancels the job; a job still running finishes detached.
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take().filter(JoinHandle::is_finished) {
            let _ = worker.join();
        }
    }
}
