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

use ketchup_analysis::fea::{FeaElementKind, FeaError, FeaModel, FeaSolveSettings};
use ketchup_analysis::fea_thermal::{
    THERMAL_MODEL_SCHEMA_V1, ThermalElement, ThermalError, ThermalMaterial, ThermalModel,
    ThermalSolveSettings, ThermalTemperature, solve_steady_conduction,
};

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
    // R1121ac P3: the matrices reserved, and the budget counted, a place for
    // each entry rather than each position, about six per position on Tet4;
    // the bar then needed 32 MiB to solve and 68 MiB for modes, now 22 and 29.
    assert!(budget <= 26 << 20, "solve needs {budget}");
    let budget = least_budget(|settings| dense.modes(settings, 1));
    assert!(budget <= 36 << 20, "modes need {budget}");
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

    // Checking the model is budgeted too (R1121ac P3): under a budget its
    // checks do not fit it is refused before them, and under one of 256 bytes
    // per element they take no more.
    for budget in [64 << 10, model.elements.len() * 256] {
        let (solved, taken) = peak(|| model.solve(settings(budget)));
        assert!(
            matches!(solved, Err(FeaError::FactorTooLarge { .. })),
            "{solved:?}",
            solved = solved.map(|_| ())
        );
        assert!(taken <= budget, "checking took {taken} under {budget}");
    }

    // So is heat conduction on the same mesh (R1121ac P3: it built its
    // entries and matrix whole before the factor's own check).
    let heat = thermal(&model);
    let (conducted, taken) = peak(|| {
        solve_steady_conduction(
            &heat,
            ThermalSolveSettings {
                maximum_factor_bytes: budget,
                ..ThermalSolveSettings::default()
            },
        )
    });
    assert!(
        matches!(conducted, Err(ThermalError::FactorTooLarge { .. })),
        "{conducted:?}"
    );
    assert!(taken <= budget + vectors(&model), "conduction took {taken}");
    // And its checks (R1122ac, review §4 P3: about 207 B per tetrahedron,
    // unbudgeted).
    for budget in [64 << 10, heat.elements.len() * 256] {
        let (conducted, taken) = peak(|| {
            solve_steady_conduction(
                &heat,
                ThermalSolveSettings {
                    maximum_factor_bytes: budget,
                    ..ThermalSolveSettings::default()
                },
            )
        });
        assert!(
            matches!(conducted, Err(ThermalError::FactorTooLarge { .. })),
            "{conducted:?}"
        );
        assert!(taken <= budget, "checking took {taken} under {budget}");
    }
}

/// The mesh of `model` conducting heat, one end held.
fn thermal(model: &FeaModel) -> ThermalModel {
    let elements = model
        .elements
        .iter()
        .filter_map(|element| match element.kind {
            FeaElementKind::LinearTetrahedron4 { nodes, .. } => Some(ThermalElement {
                id: element.id,
                nodes,
                material_id: 1,
            }),
            _ => None,
        })
        .collect();
    ThermalModel {
        schema: THERMAL_MODEL_SCHEMA_V1.into(),
        case_id: "bar".into(),
        nodes: model.nodes.clone(),
        materials: vec![ThermalMaterial {
            id: 1,
            conductivity_w_per_m_k: 50.0,
        }],
        elements,
        temperatures: vec![ThermalTemperature {
            node: 0,
            temperature_c: 20.0,
        }],
        boundaries: Vec::new(),
    }
}
