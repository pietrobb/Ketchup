//! Process-wide pool of verified exact workers. Reusing a worker keeps its
//! digest-keyed native graph outputs and pair results warm across requests.
use ketchup_scheduler::{ExactWorkerSupervisor, WorkerError};
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

const MAX_IDLE_WORKERS: usize = 2;
static IDLE: Mutex<Vec<ExactWorkerSupervisor>> = Mutex::new(Vec::new());

/// A checked-out worker. It returns to the pool only through `release`, so a
/// failed or cancelled use always discards (and thereby kills) the process.
pub struct PooledExactWorker {
    worker: Option<ExactWorkerSupervisor>,
    reusable: bool,
}

pub fn checkout(
    executable: &Path,
    cancelled: &AtomicBool,
) -> Result<PooledExactWorker, WorkerError> {
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
        None => ExactWorkerSupervisor::spawn_with_cancellation(executable, cancelled)?,
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
            if idle.len() < MAX_IDLE_WORKERS {
                idle.push(worker);
            }
        }
    }
}
