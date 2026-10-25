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

/// The least memory a worker running beside others gets of their shared budget.
const MIN_WORKER_BYTES: usize = 1 << 30;

/// Exact workers one request may run side by side here: one per two cores, at
/// least two, and no more than leave each [`MIN_WORKER_BYTES`] of the budget they
/// share. The pool keeps as many idle, so a repeated check finds each warm.
pub(crate) fn parallel_worker_limit() -> usize {
    let cores = std::thread::available_parallelism().map_or(2, usize::from);
    let affordable = ketchup_scheduler::exact_worker_memory_bytes() / MIN_WORKER_BYTES;
    (cores / 2)
        .clamp(2, MAX_PARALLEL_WORKERS)
        .min(affordable.max(1))
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

/// Idle workers of `executable` a request can take without starting one.
pub(crate) fn idle_workers(executable: Option<&Path>) -> usize {
    let Some(canonical) = executable.and_then(|path| path.canonicalize().ok()) else {
        return 0;
    };
    IDLE.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|worker| worker.executable() == canonical.as_path())
        .count()
}

/// A reusable idle worker of `executable` that is one of `workers` side by side: its
/// memory limit is at most their share and more than half of it, so they stay within
/// one worker's budget and none gets much less than its share. A discarded one is
/// killed by its client's Drop.
fn take_idle(executable: &Path, workers: usize) -> Option<ExactWorkerSupervisor> {
    let canonical = executable.canonicalize().ok();
    let share = ketchup_scheduler::exact_worker_memory_bytes() / workers.max(1);
    loop {
        let mut worker = {
            let mut idle = IDLE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let index = idle.iter().rposition(|worker| {
                Some(worker.executable()) == canonical.as_deref()
                    && worker.memory_limit() <= share
                    && worker.memory_limit() > share / 2
            })?;
            idle.remove(index)
        };
        if worker.is_reusable() {
            return Some(worker);
        }
    }
}

pub fn checkout(
    executable: Option<&Path>,
    cancelled: &AtomicBool,
) -> Result<PooledExactWorker, ExactWorkerUnavailable> {
    checkout_sharing(executable, 1, cancelled)
}

/// A worker that is one of `workers` one request runs side by side, which share one
/// worker's memory (review 2026-10-24 EX-20).
pub fn checkout_sharing(
    executable: Option<&Path>,
    workers: usize,
    cancelled: &AtomicBool,
) -> Result<PooledExactWorker, ExactWorkerUnavailable> {
    let executable = executable.ok_or(ExactWorkerUnavailable::NotFound)?;
    let worker = match take_idle(executable, workers) {
        Some(worker) => worker,
        None => ExactWorkerSupervisor::spawn_sharing(executable, workers, cancelled)
            .map_err(|cause| ExactWorkerUnavailable::Checkout { cause })?,
    };
    Ok(PooledExactWorker {
        worker: Some(worker),
        reusable: false,
    })
}

/// An already running worker of `executable` that can be one of `workers`, or none;
/// never starts one.
pub fn checkout_idle(executable: Option<&Path>, workers: usize) -> Option<PooledExactWorker> {
    take_idle(executable?, workers).map(|worker| PooledExactWorker {
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
            // The one used last is the warmest: it takes the place of the longest idle,
            // which may be of another share.
            let mut idle = IDLE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let evicted = (idle.len() >= parallel_worker_limit()).then(|| idle.remove(0));
            idle.push(worker);
            // Stopped outside the lock.
            drop(idle);
            drop(evicted);
        }
    }
}
