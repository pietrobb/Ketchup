//! Models larger than the old dense solver allowed (256 nodes, 4 096
//! elements) solve exactly where linear tetrahedra are exact.

use crate::fea_bar::{POISSON, STRESS_MPA, YOUNG_MPA, tetrahedral_bar};
use ketchup_analysis::fea::{FeaError, FeaSolveSettings};

#[test]
fn a_tetrahedral_bar_past_the_old_dense_limits_stretches_by_f_l_over_e_a() {
    let cell_mm = 10.0;
    let model = tetrahedral_bar(60, 4, cell_mm);
    assert!(model.nodes.len() > 256 && model.elements.len() > 4_096);
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    let stretch = STRESS_MPA * 600.0 / YOUNG_MPA;
    for (node, displacement) in model.nodes.iter().zip(&solution.displacements_mm) {
        let [x, y, z] = node.position_mm;
        let expected = [
            STRESS_MPA * x / YOUNG_MPA,
            -POISSON * STRESS_MPA * y / YOUNG_MPA,
            -POISSON * STRESS_MPA * z / YOUNG_MPA,
        ];
        for axis in 0..3 {
            assert!(
                (displacement[axis] - expected[axis]).abs() <= 1.0e-9 * stretch,
                "node {:?}: {:?} against {:?}",
                node.position_mm,
                displacement,
                expected
            );
        }
    }
    assert!((solution.maximum_von_mises_stress_mpa - STRESS_MPA).abs() <= 1.0e-8 * STRESS_MPA);
    assert!(
        solution
            .force_balance_n
            .iter()
            .all(|value| value.abs() <= 1.0e-6)
    );
}

#[test]
fn a_bar_supported_only_along_it_is_still_singular() {
    let mut model = tetrahedral_bar(60, 4, 10.0);
    for constraint in &mut model.constraints {
        constraint.displacement_mm = [Some(0.0), None, None];
    }
    assert_eq!(
        model.solve(FeaSolveSettings::default()),
        Err(FeaError::SingularSystem)
    );
}

#[test]
fn a_factor_past_the_memory_budget_is_refused_with_its_size() {
    let model = tetrahedral_bar(60, 4, 10.0);
    let settings = FeaSolveSettings {
        maximum_factor_bytes: 1 << 20,
        ..FeaSolveSettings::default()
    };
    let Err(FeaError::FactorTooLarge {
        needed_bytes,
        budget_bytes,
    }) = model.solve(settings)
    else {
        panic!("a factor larger than 1 MiB must be refused");
    };
    assert_eq!(budget_bytes, 1 << 20);
    assert!(needed_bytes > budget_bytes);
    // R1121ac P3: the size is of what was held when the first piece past the
    // budget was asked for, not of the whole study, and natural frequencies are
    // refused with the same words as a solve.
    let text = FeaError::FactorTooLarge {
        needed_bytes,
        budget_bytes,
    }
    .to_string();
    assert!(
        text.contains("at least") && !text.contains("the solve"),
        "{text}"
    );
    // R1122ac, review §4 P3: a budget under 1 MiB read "the 0 MiB it may use".
    let small = FeaError::FactorTooLarge {
        needed_bytes: 700 << 10,
        budget_bytes: 512 << 10,
    }
    .to_string();
    assert!(
        small.contains("at least 700 KiB") && small.contains("the 512 KiB it may use"),
        "{small}"
    );
    let large = FeaError::FactorTooLarge {
        needed_bytes: (600 << 20) + 1,
        budget_bytes: 512 << 20,
    }
    .to_string();
    assert!(
        large.contains("at least 601 MiB") && large.contains("the 512 MiB it may use"),
        "{large}"
    );
}
