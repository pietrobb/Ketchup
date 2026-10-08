//! Process-wide pool of verified exact workers. Reusing a worker keeps its
//! digest-keyed native graph outputs and pair results warm across requests.
use ketchup_scheduler::{ExactWorkerSupervisor, WorkerError};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

/// Most exact workers one request runs side by side.
const MAX_PARALLEL_WORKERS: usize = 12;

/// Exact workers one request may run side by side here: one per two cores, at
/// least two. The pool keeps as many idle, so a repeated check finds each warm.
pub(crate) fn parallel_worker_limit() -> usize {
    let cores = std::thread::available_parallelism().map_or(2, usize::from);
    (cores / 2).clamp(2, MAX_PARALLEL_WORKERS)
}
static IDLE: Mutex<Vec<ExactWorkerSupervisor>> = Mutex::new(Vec::new());

/// A checked-out worker. It returns to the pool only through `release`, so a
/// failed or cancelled use always discards (and thereby kills) the process.
pub struct PooledExactWorker {
    worker: Option<ExactWorkerSupervisor>,
    reusable: bool,
}

/// Why no exact worker serves a request.
#[derive(Debug)]
pub enum ExactWorkerUnavailable {
    /// No worker executable was found.
    NotFound,
    /// Starting or reusing the worker failed; `cause` says how.
    Checkout { cause: WorkerError },
}

impl fmt::Display for ExactWorkerUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("exact worker unavailable"),
            Self::Checkout { cause } => write!(formatter, "exact worker unavailable: {cause}"),
        }
    }
}

impl std::error::Error for ExactWorkerUnavailable {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotFound => None,
            Self::Checkout { cause } => Some(cause),
        }
    }
}

#[cfg(test)]
pub(crate) fn idle_count() -> usize {
    IDLE.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len()
}

pub fn checkout(
    executable: Option<&Path>,
    cancelled: &AtomicBool,
) -> Result<PooledExactWorker, ExactWorkerUnavailable> {
    let executable = executable.ok_or(ExactWorkerUnavailable::NotFound)?;
    let canonical = executable.canonicalize().ok();
    let idle = {
        let mut idle = IDLE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        idle.iter()
            .rposition(|worker| Some(worker.executable()) == canonical.as_deref())
            .map(|index| idle.remove(index))
    };
    // A discarded idle worker is killed by its client's Drop.
    let worker = match idle.and_then(|mut worker| worker.is_reusable().then_some(worker)) {
        Some(worker) => worker,
        None => ExactWorkerSupervisor::spawn_with_cancellation(executable, cancelled)
            .map_err(|cause| ExactWorkerUnavailable::Checkout { cause })?,
    };
    Ok(PooledExactWorker {
        worker: Some(worker),
        reusable: false,
    })
}

impl PooledExactWorker {
    /// Return the worker to the pool. Call only after a fully successful use.
    pub fn release(mut self) {
        self.reusable = true;
    }
}

impl Deref for PooledExactWorker {
    type Target = ExactWorkerSupervisor;
    fn deref(&self) -> &Self::Target {
        self.worker.as_ref().expect("worker present until drop")
    }
}

impl DerefMut for PooledExactWorker {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.worker.as_mut().expect("worker present until drop")
    }
}

impl Drop for PooledExactWorker {
    fn drop(&mut self) {
        let Some(mut worker) = self.worker.take() else {
            return;
        };
        if self.reusable && worker.is_reusable() {
            let mut idle = IDLE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if idle.len() < parallel_worker_limit() {
                idle.push(worker);
            }
        }
    }
}
