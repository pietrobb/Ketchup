//! R1120ac-FEA-902: a solve or modes study in the window assembled far more
//! than the memory budget before refusing, and modes held the mass, filtered
//! copies and several matrices beside the factor (2.8 times the budget the
//! check before assembly let through). The memory taken is counted here by the
//! allocator of this test, a binary of its own, as the counts are of the whole
//! process.
// The allocator only counts what it passes on to the system's.
#![allow(unsafe_code)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use ketchup_analysis::fea::{FeaError, FeaModel, FeaSolveSettings};

#[allow(dead_code)]
mod fea_bar;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        // SAFETY: the caller's layout is passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` was allocated by `System` with this layout, in `alloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The memory `run` takes above what is held before it, at most.
fn peak<T>(run: impl FnOnce() -> T) -> (T, usize) {
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let value = run();
    (value, PEAK.load(Ordering::Relaxed) - before)
}

fn settings(budget_bytes: usize) -> FeaSolveSettings {
    FeaSolveSettings {
        maximum_factor_bytes: budget_bytes,
        ..FeaSolveSettings::default()
    }
}

/// What a study may take beyond its budget: vectors of one value per unknown.
fn vectors(model: &FeaModel) -> usize {
    64 * 3 * model.nodes.len() * size_of::<f64>()
}

/// The smallest budget, to a sixteenth, under which `study` succeeds.
fn least_budget<T>(study: impl Fn(FeaSolveSettings) -> Result<T, FeaError>) -> usize {
    let (mut refused, mut accepted) = (0, 1 << 27);
    assert!(study(settings(accepted)).is_ok());
    while accepted - refused > accepted / 16 {
        let middle = refused + (accepted - refused) / 2;
        match study(settings(middle)) {
            Ok(_) => accepted = middle,
            Err(FeaError::FactorTooLarge {
                needed_bytes,
                budget_bytes,
            }) => {
                assert_eq!(budget_bytes, middle);
                assert!(needed_bytes > middle);
                refused = middle;
            }
            Err(error) => panic!("{error:?}"),
        }
    }
    accepted
}

/// One test, so that no other test allocates beside the counts.
#[test]
fn solve_and_modes_stay_within_the_memory_budget() {
    let model = fea_bar::tetrahedral_bar(60, 4, 10.0);
    let mut dense = model.clone();
    dense.materials[0].density_kg_per_m3 = Some(2_700.0);

    // Under the least budget each study succeeds with, and under one barely
    // too small, it never holds more than the budget (and a few vectors).
    let budget = least_budget(|settings| model.solve(settings));
    for budget in [budget, budget - budget / 16] {
        let (result, taken) = peak(|| model.solve(settings(budget)));
        eprintln!("solve under {budget}: took {taken}");
        assert!(
            taken <= budget + vectors(&model),
            "solve took {taken} under {budget} ({result:?})",
            result = result.map(|_| ())
        );
    }
    let budget = least_budget(|settings| dense.modes(settings, 1));
    for budget in [budget, budget - budget / 16] {
        let (result, taken) = peak(|| dense.modes(settings(budget), 1));
        eprintln!("modes under {budget}: took {taken}");
        assert!(
            taken <= budget + vectors(&dense),
            "modes took {taken} under {budget} ({result:?})",
            result = result.map(|_| ())
        );
    }

    // A model far past the budget is refused before it is assembled.
    let budget = 1 << 20;
    let (solved, taken) = peak(|| model.solve(settings(budget)));
    assert!(matches!(solved, Err(FeaError::FactorTooLarge { .. })));
    assert!(taken <= budget + vectors(&model), "solve took {taken}");
    let (modes, taken) = peak(|| dense.modes(settings(budget), 1));
    assert!(matches!(modes, Err(FeaError::FactorTooLarge { .. })));
    assert!(taken <= budget + vectors(&dense), "modes took {taken}");
}
