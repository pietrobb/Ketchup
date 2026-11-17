//! One memory budget for every background process of the window: exact workers,
//! program workers (programs, DWG and SketchUp files, underlays) and plugins. Each
//! keeps the memory limit of its kind; the budget bounds what they use together.
//! A watch measures what each process uses now, every [`WATCH_EVERY`]; past the
//! budget it stops idle processes first, then the largest, until the rest fit. The
//! request of a stopped process fails with [`BudgetStop`], which says what it and
//! the others used. No job waits for memory, so none can wait on another forever.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

/// How often the window's budget is measured; what is written between two
/// measurements may pass it.
pub const WATCH_EVERY: Duration = Duration::from_millis(250);

/// One process the budget measures.
pub trait ProcessMemory: Send + Sync {
    /// Bytes it uses now; `None` once it has ended.
    fn bytes(&self) -> Option<u64>;
    /// Ends it.
    fn stop(&self);
}

impl ProcessMemory for process_wrap::std::ProcessMeter {
    fn bytes(&self) -> Option<u64> {
        Self::bytes(self)
    }

    fn stop(&self) {
        self.kill();
    }
}

/// Why the budget stopped a process in the middle of a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetStop {
    /// Bytes the stopped process used.
    pub used: u64,
    /// Bytes the other processes of the budget used then.
    pub others: u64,
    /// The budget.
    pub total: u64,
}

impl fmt::Display for BudgetStop {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the background jobs of the window together needed more than its memory budget \
             of {:.1} GiB; this one used {:.1} GiB and the others {:.1} GiB, so it was \
             stopped. Let the other large imports, exports or programs finish, or cancel \
             them, and try again",
            gibibytes(self.total),
            gibibytes(self.used),
            gibibytes(self.others)
        )
    }
}

impl std::error::Error for BudgetStop {}

/// `bytes` in GiB, to a MiB.
fn gibibytes(bytes: u64) -> f64 {
    f64::from(u32::try_from(bytes >> 20).unwrap_or(u32::MAX)) / 1024.0
}

pub struct MemoryBudget {
    total: u64,
    processes: Mutex<Vec<Arc<Tracked>>>,
}

struct Tracked {
    meter: Box<dyn ProcessMemory>,
    busy: AtomicBool,
    /// Set when the budget stopped it, with whether it was in a request then.
    stopped: Mutex<Option<(BudgetStop, bool)>>,
}

/// One process in a budget; it leaves the budget when this is dropped.
pub struct Registration {
    budget: Arc<MemoryBudget>,
    tracked: Arc<Tracked>,
}

/// The process is in a request until this is dropped.
pub struct Busy(Arc<Tracked>);

/// Memory the exact workers of one request may commit: half of this computer's, so a
/// hostile or huge STEP or IGES file stops the worker instead of exhausting the system
/// and the window with it. Workers running side by side share it
/// ([`crate::ExactWorkerSupervisor::spawn_sharing`]). 4 GiB when the system does not
/// say how much it has.
pub fn exact_worker_memory_bytes() -> usize {
    static BYTES: OnceLock<usize> = OnceLock::new();
    *BYTES.get_or_init(|| {
        process_wrap::std::physical_memory_bytes()
            .and_then(|bytes| usize::try_from(bytes / 2).ok())
            .unwrap_or(4 << 30)
    })
}

/// The budget of this window: [`exact_worker_memory_bytes`], half of this computer's
/// memory, watched by a thread of its own.
pub fn window_memory_budget() -> &'static Arc<MemoryBudget> {
    static BUDGET: OnceLock<Arc<MemoryBudget>> = OnceLock::new();
    BUDGET.get_or_init(|| {
        MemoryBudget::watched(u64::try_from(exact_worker_memory_bytes()).unwrap_or(u64::MAX))
    })
}

/// `child` in `budget`, or nowhere when this system does not let its memory be read:
/// it still has the limit of its kind.
pub(crate) fn track(
    budget: &Arc<MemoryBudget>,
    child: &dyn process_wrap::std::ChildWrapper,
) -> Option<Registration> {
    process_wrap::std::ProcessMeter::of(child)
        .ok()
        .map(|meter| budget.register(Box::new(meter)))
}

impl MemoryBudget {
    /// A budget that is enforced only when [`Self::enforce`] is called.
    #[must_use]
    pub fn new(total: u64) -> Arc<Self> {
        Arc::new(Self {
            total,
            processes: Mutex::new(Vec::new()),
        })
    }

    /// A budget a thread enforces every [`WATCH_EVERY`] while it exists.
    #[must_use]
    pub fn watched(total: u64) -> Arc<Self> {
        let budget = Self::new(total);
        let watched = Arc::downgrade(&budget);
        std::thread::spawn(move || {
            while let Some(budget) = watched.upgrade() {
                budget.enforce();
                drop(budget);
                std::thread::sleep(WATCH_EVERY);
            }
        });
        budget
    }

    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn register(self: &Arc<Self>, meter: Box<dyn ProcessMemory>) -> Registration {
        let tracked = Arc::new(Tracked {
            meter,
            busy: AtomicBool::new(false),
            stopped: Mutex::new(None),
        });
        self.lock().push(Arc::clone(&tracked));
        Registration {
            budget: Arc::clone(self),
            tracked,
        }
    }

    /// Measures every process; while together they use more than the budget, stops
    /// idle ones before busy ones and larger before smaller. Returns how many it stopped.
    pub fn enforce(&self) -> usize {
        let processes = self.lock();
        let mut running: Vec<(&Arc<Tracked>, u64)> = processes
            .iter()
            .filter(|tracked| tracked.stop_state().is_none())
            .filter_map(|tracked| Some((tracked, tracked.meter.bytes()?)))
            .collect();
        let mut sum = running.iter().map(|(_, bytes)| bytes).sum::<u64>();
        let mut stopped = 0;
        while sum > self.total {
            let Some(index) = running
                .iter()
                .enumerate()
                .max_by_key(|(_, (tracked, bytes))| (!tracked.busy.load(Ordering::Acquire), *bytes))
                .map(|(index, _)| index)
            else {
                break;
            };
            let (tracked, used) = running.swap_remove(index);
            let stop = BudgetStop {
                used,
                others: sum - used,
                total: self.total,
            };
            // Recorded before the stop, so the request that sees its process end finds why.
            *tracked
                .stopped
                .lock()
                .unwrap_or_else(PoisonError::into_inner) =
                Some((stop, tracked.busy.load(Ordering::Acquire)));
            tracked.meter.stop();
            sum -= used;
            stopped += 1;
        }
        stopped
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<Tracked>>> {
        self.processes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Tracked {
    fn stop_state(&self) -> Option<(BudgetStop, bool)> {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Registration {
    /// Marks the process as in a request until the returned guard is dropped.
    #[must_use]
    pub fn busy(&self) -> Busy {
        self.tracked.busy.store(true, Ordering::Release);
        Busy(Arc::clone(&self.tracked))
    }

    /// Why the budget stopped the process in the middle of a request; `None` when it
    /// did not, or stopped it idle, which no request is to blame for.
    #[must_use]
    pub fn stopped(&self) -> Option<BudgetStop> {
        match self.tracked.stop_state() {
            Some((stop, true)) => Some(stop),
            _ => None,
        }
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.busy.store(false, Ordering::Release);
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.budget
            .lock()
            .retain(|tracked| !Arc::ptr_eq(tracked, &self.tracked));
    }
}

impl fmt::Debug for Registration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Registration")
            .field("busy", &self.tracked.busy.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "memory_budget_tests.rs"]
mod tests;
