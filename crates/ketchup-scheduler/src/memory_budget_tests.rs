use super::*;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc;

/// A process whose use the test sets; stopping it ends it.
struct Fake {
    bytes: AtomicU64,
    killed: AtomicBool,
}

impl ProcessMemory for Arc<Fake> {
    fn bytes(&self) -> Option<u64> {
        (!self.killed.load(Ordering::Acquire)).then(|| self.bytes.load(Ordering::Acquire))
    }

    fn stop(&self) {
        self.killed.store(true, Ordering::Release);
    }
}

fn fake(budget: &Arc<MemoryBudget>, bytes: u64) -> (Arc<Fake>, Registration) {
    let process = Arc::new(Fake {
        bytes: AtomicU64::new(bytes),
        killed: AtomicBool::new(false),
    });
    let registration = budget.register(Box::new(Arc::clone(&process)));
    (process, registration)
}

fn killed(process: &Fake) -> bool {
    process.killed.load(Ordering::Acquire)
}

#[test]
fn processes_within_the_budget_together_keep_running() {
    let budget = MemoryBudget::new(1000);
    let (first, _first) = fake(&budget, 600);
    let (second, _second) = fake(&budget, 400);
    assert_eq!(budget.enforce(), 0);
    assert!(!killed(&first) && !killed(&second));
}

#[test]
fn past_the_budget_the_largest_busy_process_is_stopped_with_what_each_used() {
    let budget = MemoryBudget::new(1000);
    let (small, small_registration) = fake(&budget, 300);
    let (large, large_registration) = fake(&budget, 900);
    let _small_busy = small_registration.busy();
    let _large_busy = large_registration.busy();
    assert_eq!(budget.enforce(), 1);
    assert!(killed(&large) && !killed(&small));
    assert_eq!(
        large_registration.stopped(),
        Some(BudgetStop {
            used: 900,
            others: 300,
            total: 1000
        })
    );
    assert_eq!(small_registration.stopped(), None);
    // The stopped one no longer counts, so the other keeps running.
    assert_eq!(budget.enforce(), 0);
}

#[test]
fn idle_processes_are_stopped_before_busy_ones_and_their_request_is_not_blamed() {
    let budget = MemoryBudget::new(1000);
    let (idle, idle_registration) = fake(&budget, 200);
    let (busy, busy_registration) = fake(&budget, 900);
    let _busy = busy_registration.busy();
    assert_eq!(budget.enforce(), 1);
    assert!(killed(&idle) && !killed(&busy));
    // Nobody was waiting on the idle one: its next request finds it ended and starts another.
    assert_eq!(idle_registration.stopped(), None);
    assert_eq!(busy_registration.stopped(), None);
}

#[test]
fn as_many_are_stopped_as_bring_the_window_back_within_its_budget() {
    let budget = MemoryBudget::new(1000);
    let (first, first_registration) = fake(&budget, 700);
    let (second, second_registration) = fake(&budget, 600);
    let (third, third_registration) = fake(&budget, 500);
    let _busy = [
        first_registration.busy(),
        second_registration.busy(),
        third_registration.busy(),
    ];
    assert_eq!(budget.enforce(), 2);
    assert!(killed(&first) && killed(&second) && !killed(&third));
    assert_eq!(
        second_registration.stopped(),
        Some(BudgetStop {
            used: 600,
            others: 500,
            total: 1000
        })
    );
}

#[test]
fn an_ended_process_counts_nothing_and_a_dropped_one_leaves_the_budget() {
    let budget = MemoryBudget::new(1000);
    let (ended, _ended) = fake(&budget, 900);
    ended.stop();
    let (dropped, dropped_registration) = fake(&budget, 900);
    drop(dropped_registration);
    let (running, _running) = fake(&budget, 500);
    assert_eq!(budget.enforce(), 0);
    assert!(!killed(&dropped) && !killed(&running));
}

#[test]
fn the_watch_stops_a_process_past_the_budget_on_its_own() {
    /// A process past the budget alone, which says when it is stopped.
    struct Reporting(Mutex<mpsc::Sender<()>>);

    impl ProcessMemory for Reporting {
        fn bytes(&self) -> Option<u64> {
            Some(2000)
        }

        fn stop(&self) {
            self.0.lock().unwrap().send(()).unwrap();
        }
    }

    let budget = MemoryBudget::watched(1000);
    let (stops, stopped) = mpsc::channel();
    let _registration = budget.register(Box::new(Reporting(Mutex::new(stops))));
    stopped.recv_timeout(Duration::from_secs(60)).unwrap();
}

#[test]
fn a_stop_says_what_the_process_and_the_others_used_of_the_budget() {
    let stop = BudgetStop {
        used: 6 << 30,
        others: 3 << 29,
        total: 8 << 30,
    };
    assert_eq!(
        stop.to_string(),
        "the background jobs of the window together needed more than its memory budget of \
         8.0 GiB; this one used 6.0 GiB and the others 1.5 GiB, so it was stopped. Let the \
         other large imports, exports or programs finish, or cancel them, and try again"
    );
}

/// Alone past the budget, the advice is not to wait for the others.
#[test]
fn a_job_alone_past_the_budget_is_not_told_to_wait_for_others() {
    let stop = BudgetStop {
        used: 9 << 30,
        others: 0,
        total: 8 << 30,
    };
    let text = stop.to_string();
    assert!(text.contains("alone") && text.contains("Split"), "{text}");
    assert!(!text.contains("finish"), "{text}");
}

fn files(files: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let files = files
        .iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect::<std::collections::HashMap<_, _>>();
    move |path| files.get(path).cloned()
}

/// In a container the budget is a share of its limit, not of the host's memory.
#[test]
fn the_container_limit_bounds_the_budget() {
    // cgroup v2: the least of the group and its ancestors; "max" is no limit.
    let nested = files(&[
        ("/proc/self/cgroup", "0::/ci/job\n"),
        ("/sys/fs/cgroup/ci/job/memory.max", "max\n"),
        ("/sys/fs/cgroup/ci/memory.max", "2147483648\n"),
    ]);
    assert_eq!(cgroup_memory_limit(nested), Some(2 << 30));
    let container = files(&[
        ("/proc/self/cgroup", "0::/\n"),
        ("/sys/fs/cgroup/memory.max", "1073741824\n"),
    ]);
    assert_eq!(cgroup_memory_limit(container), Some(1 << 30));
    let unlimited = files(&[
        ("/proc/self/cgroup", "0::/user.slice\n"),
        ("/sys/fs/cgroup/user.slice/memory.max", "max\n"),
    ]);
    assert_eq!(cgroup_memory_limit(unlimited), None);
    // cgroup v1.
    let v1 = files(&[
        ("/proc/self/cgroup", "4:memory:/docker/abc\n"),
        ("/sys/fs/cgroup/memory/memory.limit_in_bytes", "536870912\n"),
    ]);
    assert_eq!(cgroup_memory_limit(v1), Some(1 << 29));
    assert_eq!(cgroup_memory_limit(files(&[])), None);
}
