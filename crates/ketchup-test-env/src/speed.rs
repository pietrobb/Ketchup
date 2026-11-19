//! The `perf_*` budgets, held to the speed of the machine the tests run on
//! (review 2026-11-19 og, CI-807): the nightly runner is not equally fast every
//! night, and a frame of 401 ms against 400 failed a night the code had not
//! changed.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The fastest of five runs of [`reference_work`] on the machine the budgets
/// are measured on (24 cores, 2026-11-19), in the build the test is in.
const REFERENCE: Duration = Duration::from_millis(if cfg!(debug_assertions) { 144 } else { 21 });

/// A slower machine gets this many times its budgets at most, so a stalled
/// runner still fails a regression of several times.
const MOST_SLOWDOWN: f64 = 4.0;

/// How many times slower than the reference machine this one does a fixed
/// piece of work, from 1 (as fast or faster) to [`MOST_SLOWDOWN`]. Measured
/// once per test process.
pub fn perf_slowdown() -> f64 {
    static SLOWDOWN: OnceLock<f64> = OnceLock::new();
    *SLOWDOWN.get_or_init(|| slowdown((0..5).map(|_| reference_work()).min().unwrap_or(REFERENCE)))
}

/// How many times [`REFERENCE`] `fastest` is, from 1 to [`MOST_SLOWDOWN`].
fn slowdown(fastest: Duration) -> f64 {
    (fastest.as_secs_f64() / REFERENCE.as_secs_f64()).clamp(1.0, MOST_SLOWDOWN)
}

/// `budget`, set on the reference machine, for this machine.
pub fn perf_budget(budget: Duration) -> Duration {
    budget.mul_f64(perf_slowdown())
}

/// One thread walking a 4 MiB table in a pseudo-random order with a square
/// root on each step: the memory and floating-point work a frame does.
pub fn reference_work() -> Duration {
    const SLOTS: usize = 1 << 19;
    let start = Instant::now();
    let mut table: Vec<u64> = (0..SLOTS as u64).collect();
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut sum = 0.0_f64;
    for _ in 0..8_000_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let slot = (state as usize) & (SLOTS - 1);
        table[slot] = table[slot].wrapping_add(state);
        sum += (table[slot] as f64).sqrt();
    }
    std::hint::black_box((table, sum));
    start.elapsed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_is_never_shortened_and_grows_at_most_four_times() {
        assert_eq!(slowdown(REFERENCE / 2), 1.0);
        assert_eq!(slowdown(REFERENCE), 1.0);
        assert_eq!(slowdown(REFERENCE * 3), 3.0);
        assert_eq!(slowdown(REFERENCE * 10), MOST_SLOWDOWN);
    }

    /// The nightly log shows how fast the runner was.
    #[test]
    #[ignore = "perf: release timing budget, runs nightly"]
    fn perf_the_machines_slowdown_is_measured() {
        let runs: Vec<_> = (0..5).map(|_| reference_work()).collect();
        let slowdown = perf_slowdown();
        println!("reference work {runs:?} against {REFERENCE:?}: slowdown {slowdown}");
        assert!((1.0..=MOST_SLOWDOWN).contains(&slowdown), "{slowdown}");
    }
}
