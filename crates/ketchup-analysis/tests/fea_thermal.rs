//! Steady heat conduction on linear tetrahedra. A bar whose sides are
//! insulated carries heat along its length only, so its temperature is
//! linear and the elements reproduce it exactly: held ends, a heat flux into
//! one end and a convecting end all have closed-form answers.

use ketchup_analysis::fea::FeaNode;
use ketchup_analysis::fea_thermal::{
    THERMAL_MODEL_SCHEMA_V1, ThermalBoundary, ThermalElement, ThermalError, ThermalMaterial,
    ThermalModel, ThermalSolveSettings, ThermalTemperature, solve_steady_conduction,
};
use std::collections::BTreeMap;

/// Steel, W/(m·K).
const CONDUCTIVITY: f64 = 50.0;
const CELL_MM: f64 = 10.0;
const CELLS: usize = 12;
const SIDE: usize = 2;
const LENGTH_MM: f64 = CELL_MM * 12.0;
const AREA_MM2: f64 = (CELL_MM * 2.0) * (CELL_MM * 2.0);

fn index(x: usize, y: usize, z: usize) -> usize {
    (x * (SIDE + 1) + y) * (SIDE + 1) + z
}

/// A bar of `CELLS` × `SIDE` × `SIDE` cubes along `x`, each cut into six
/// tetrahedra around its main diagonal, with no boundary conditions yet.
fn bar() -> ThermalModel {
    let mut nodes = Vec::new();
    for x in 0..=CELLS {
        for y in 0..=SIDE {
            for z in 0..=SIDE {
                nodes.push(FeaNode {
                    position_mm: [x, y, z]
                        .map(|value| CELL_MM * f64::from(u32::try_from(value).unwrap())),
                });
            }
        }
    }
    let mut elements = Vec::new();
    for x in 0..CELLS {
        for y in 0..SIDE {
            for z in 0..SIDE {
                let corner = |bits: [usize; 3]| index(x + bits[0], y + bits[1], z + bits[2]);
                for axes in [
                    [0, 1, 2],
                    [0, 2, 1],
                    [1, 0, 2],
                    [1, 2, 0],
                    [2, 0, 1],
                    [2, 1, 0],
                ] {
                    let mut bits = [0; 3];
                    let mut path = [corner(bits); 4];
                    for (step, axis) in axes.into_iter().enumerate() {
                        bits[axis] = 1;
                        path[step + 1] = corner(bits);
                    }
                    elements.push(ThermalElement {
                        id: u64::try_from(elements.len()).unwrap() + 1,
                        nodes: path,
                        material_id: 7,
                    });
                }
            }
        }
    }
    ThermalModel {
        schema: THERMAL_MODEL_SCHEMA_V1.into(),
        case_id: "insulated-bar".into(),
        nodes,
        materials: vec![ThermalMaterial {
            id: 7,
            conductivity_w_per_m_k: CONDUCTIVITY,
        }],
        elements,
        temperatures: Vec::new(),
        boundaries: Vec::new(),
    }
}

/// The boundary triangles of the bar lying in the end plane `x`.
fn end_faces(model: &ThermalModel, x_mm: f64) -> Vec<[usize; 3]> {
    let mut count = BTreeMap::<[usize; 3], usize>::new();
    for element in &model.elements {
        for skipped in 0..4 {
            let mut face = [0; 3];
            let mut slot = 0;
            for (position, node) in element.nodes.into_iter().enumerate() {
                if position != skipped {
                    face[slot] = node;
                    slot += 1;
                }
            }
            face.sort_unstable();
            *count.entry(face).or_default() += 1;
        }
    }
    count
        .into_iter()
        .filter(|(face, seen)| {
            *seen == 1
                && face
                    .iter()
                    .all(|node| model.nodes[*node].position_mm[0] == x_mm)
        })
        .map(|(face, _)| face)
        .collect()
}

fn hold_end(model: &mut ThermalModel, x: usize, temperature_c: f64) {
    for y in 0..=SIDE {
        for z in 0..=SIDE {
            model.temperatures.push(ThermalTemperature {
                node: index(x, y, z),
                temperature_c,
            });
        }
    }
}

fn assert_linear(model: &ThermalModel, temperatures: &[f64], cold_c: f64, hot_c: f64) {
    for (node, temperature) in model.nodes.iter().zip(temperatures) {
        let expected = cold_c + (hot_c - cold_c) * node.position_mm[0] / LENGTH_MM;
        assert!(
            (temperature - expected).abs() < 1e-9,
            "{temperature} at {:?}, expected {expected}",
            node.position_mm
        );
    }
}

#[test]
fn a_bar_held_at_two_temperatures_carries_k_a_delta_t_over_l() {
    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    hold_end(&mut model, CELLS, 120.0);

    let solution = solve_steady_conduction(&model, ThermalSolveSettings::default()).unwrap();

    assert_linear(&model, &solution.temperatures_c, 20.0, 120.0);
    // q = k ΔT / L, in W/m² with L in metres.
    let flux = CONDUCTIVITY * 100.0 / (LENGTH_MM / 1000.0);
    let power_w = flux * AREA_MM2 * 1e-6;
    assert!((solution.heat_into_held_nodes_w - power_w).abs() < 1e-9 * power_w.max(1.0));
    assert!((solution.heat_out_of_held_nodes_w - power_w).abs() < 1e-9 * power_w.max(1.0));
    assert!(solution.heat_balance_w.abs() < 1e-9);
    for result in &solution.element_results {
        assert!((result.heat_flux_w_per_m2[0] + flux).abs() < 1e-6 * flux);
        assert!(result.heat_flux_w_per_m2[1].abs() < 1e-6 * flux);
        assert!(result.heat_flux_w_per_m2[2].abs() < 1e-6 * flux);
    }
    assert!((solution.minimum_temperature_c - 20.0).abs() < 1e-9);
    assert!((solution.maximum_temperature_c - 120.0).abs() < 1e-9);
    assert_eq!(solution.model_digest.len(), 64);
}

#[test]
fn two_materials_in_series_share_one_heat_flow() {
    let mut model = bar();
    model.materials.push(ThermalMaterial {
        id: 8,
        conductivity_w_per_m_k: 1.5,
    });
    let half = CELLS / 2;
    let per_cell = 6 * SIDE * SIDE;
    for element in model.elements.iter_mut().skip(half * per_cell) {
        element.material_id = 8;
    }
    hold_end(&mut model, 0, 20.0);
    hold_end(&mut model, CELLS, 120.0);

    let solution = solve_steady_conduction(&model, ThermalSolveSettings::default()).unwrap();

    // q = ΔT / (L₁/k₁ + L₂/k₂); the steel half drops q L₁ / k₁.
    let half_m = LENGTH_MM / 2000.0;
    let flux = 100.0 / (half_m / CONDUCTIVITY + half_m / 1.5);
    let joint = 20.0 + flux * half_m / CONDUCTIVITY;
    for (node, temperature) in model.nodes.iter().zip(&solution.temperatures_c) {
        let x = node.position_mm[0] / (LENGTH_MM / 2.0);
        let expected = if x <= 1.0 {
            20.0 + (joint - 20.0) * x
        } else {
            joint + (120.0 - joint) * (x - 1.0)
        };
        assert!((temperature - expected).abs() < 1e-9);
    }
    let power_w = flux * AREA_MM2 * 1e-6;
    assert!((solution.heat_into_held_nodes_w - power_w).abs() < 1e-9 * power_w.max(1.0));
}

#[test]
fn a_heat_flux_into_one_end_raises_it_by_q_l_over_k() {
    let mut model = bar();
    hold_end(&mut model, 0, 15.0);
    let flux = 25_000.0;
    for nodes in end_faces(&model, LENGTH_MM) {
        model.boundaries.push(ThermalBoundary::HeatFlux {
            nodes,
            flux_w_per_m2: flux,
        });
    }

    let solution = solve_steady_conduction(&model, ThermalSolveSettings::default()).unwrap();

    let rise = flux * (LENGTH_MM / 1000.0) / CONDUCTIVITY;
    assert_linear(&model, &solution.temperatures_c, 15.0, 15.0 + rise);
    let power_w = flux * AREA_MM2 * 1e-6;
    assert!((solution.heat_out_of_held_nodes_w - power_w).abs() < 1e-9);
    assert!(solution.heat_balance_w.abs() < 1e-9);
}

#[test]
fn a_convecting_end_settles_where_conduction_meets_the_film() {
    let mut model = bar();
    hold_end(&mut model, 0, 200.0);
    let film = 400.0;
    let ambient = 25.0;
    for nodes in end_faces(&model, LENGTH_MM) {
        model.boundaries.push(ThermalBoundary::Convection {
            nodes,
            film_coefficient_w_per_m2_k: film,
            ambient_c: ambient,
        });
    }

    let solution = solve_steady_conduction(&model, ThermalSolveSettings::default()).unwrap();

    // k (T0 − T_L) / L = h (T_L − T∞).
    let conductance = CONDUCTIVITY / (LENGTH_MM / 1000.0);
    let end = (conductance * 200.0 + film * ambient) / (conductance + film);
    assert_linear(&model, &solution.temperatures_c, 200.0, end);
    let power_w = film * (end - ambient) * AREA_MM2 * 1e-6;
    assert!((solution.heat_into_held_nodes_w - power_w).abs() < 1e-9);
    assert!(solution.heat_balance_w.abs() < 1e-9);
}

#[test]
fn convection_alone_fixes_the_level_without_held_temperatures() {
    let mut model = bar();
    for nodes in end_faces(&model, 0.0) {
        model.boundaries.push(ThermalBoundary::Convection {
            nodes,
            film_coefficient_w_per_m2_k: 100.0,
            ambient_c: 30.0,
        });
    }
    for nodes in end_faces(&model, LENGTH_MM) {
        model.boundaries.push(ThermalBoundary::HeatFlux {
            nodes,
            flux_w_per_m2: 0.0,
        });
    }

    let solution = solve_steady_conduction(&model, ThermalSolveSettings::default()).unwrap();

    for temperature in &solution.temperatures_c {
        assert!((temperature - 30.0).abs() < 1e-9);
    }
}

#[test]
fn a_body_with_no_held_temperature_or_film_is_refused() {
    let mut model = bar();
    for nodes in end_faces(&model, LENGTH_MM) {
        model.boundaries.push(ThermalBoundary::HeatFlux {
            nodes,
            flux_w_per_m2: 1000.0,
        });
    }
    assert_eq!(
        solve_steady_conduction(&model, ThermalSolveSettings::default()),
        Err(ThermalError::NoReferenceTemperature)
    );
}

#[test]
fn invalid_inputs_are_refused_by_name() {
    let settings = ThermalSolveSettings::default();

    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    model.materials[0].conductivity_w_per_m_k = 0.0;
    assert_eq!(
        solve_steady_conduction(&model, settings),
        Err(ThermalError::InvalidMaterial)
    );

    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    model.temperatures.push(ThermalTemperature {
        node: index(0, 0, 0),
        temperature_c: 30.0,
    });
    assert_eq!(
        solve_steady_conduction(&model, settings),
        Err(ThermalError::ConflictingTemperature)
    );

    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    model.temperatures[0].temperature_c = -300.0;
    assert_eq!(
        solve_steady_conduction(&model, settings),
        Err(ThermalError::InvalidTemperature)
    );

    // A face between two tetrahedra inside the bar.
    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    let mut inner = [index(1, 0, 0), index(1, 1, 0), index(1, 1, 1)];
    inner.sort_unstable();
    model.boundaries.push(ThermalBoundary::HeatFlux {
        nodes: inner,
        flux_w_per_m2: 10.0,
    });
    assert_eq!(
        solve_steady_conduction(&model, settings),
        Err(ThermalError::NonBoundaryFace)
    );

    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    let face = end_faces(&model, LENGTH_MM)[0];
    model.boundaries.push(ThermalBoundary::Convection {
        nodes: face,
        film_coefficient_w_per_m2_k: -5.0,
        ambient_c: 20.0,
    });
    assert_eq!(
        solve_steady_conduction(&model, settings),
        Err(ThermalError::InvalidBoundary)
    );

    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    model.elements[3].material_id = 99;
    assert_eq!(
        solve_steady_conduction(&model, settings),
        Err(ThermalError::MissingMaterial { element: 4 })
    );

    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    model.elements[5].nodes[3] = model.elements[5].nodes[0];
    assert_eq!(
        solve_steady_conduction(&model, settings),
        Err(ThermalError::RepeatedElementNode { element: 6 })
    );
}

#[test]
fn a_factor_past_the_memory_budget_is_refused_with_its_size() {
    let mut model = bar();
    hold_end(&mut model, 0, 20.0);
    let settings = ThermalSolveSettings {
        maximum_factor_bytes: 64,
        ..ThermalSolveSettings::default()
    };
    let Err(ThermalError::FactorTooLarge {
        needed_bytes,
        budget_bytes,
    }) = solve_steady_conduction(&model, settings)
    else {
        panic!("expected the factor to be refused");
    };
    assert_eq!(budget_bytes, 64);
    assert!(needed_bytes > 64);
}
